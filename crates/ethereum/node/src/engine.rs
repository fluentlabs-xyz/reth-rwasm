//! Validates execution payload wrt Ethereum Execution Engine API version.

use alloy_rpc_types_engine::ExecutionData;
pub use alloy_rpc_types_engine::{
    ExecutionPayloadEnvelopeV2, ExecutionPayloadEnvelopeV3, ExecutionPayloadEnvelopeV4,
    ExecutionPayloadV1, PayloadAttributes as EthPayloadAttributes,
};
use reth_chainspec::{EthChainSpec, EthereumHardforks};
use reth_engine_primitives::{EngineApiValidator, PayloadValidator};
use reth_ethereum_payload_builder::EthereumExecutionPayloadValidator;
use reth_ethereum_primitives::Block;
use reth_node_api::PayloadTypes;
use reth_payload_primitives::{
    validate_execution_requests, validate_version_specific_fields, EngineApiMessageVersion,
    EngineObjectValidationError, NewPayloadError, PayloadOrAttributes,
};
use reth_primitives_traits::SealedBlock;
use std::sync::Arc;

/// Validator for the ethereum engine API.
#[derive(Debug, Clone)]
pub struct EthereumEngineValidator<ChainSpec = reth_chainspec::ChainSpec> {
    inner: EthereumExecutionPayloadValidator<ChainSpec>,
}

impl<ChainSpec> EthereumEngineValidator<ChainSpec> {
    /// Instantiates a new validator.
    pub const fn new(chain_spec: Arc<ChainSpec>) -> Self {
        Self { inner: EthereumExecutionPayloadValidator::new(chain_spec) }
    }

    /// Overrides the chain-specific extra-data limit. Ethereum defaults to 32 bytes.
    pub fn with_max_extra_data_size(mut self, max: usize) -> Self {
        self.inner = self.inner.with_max_extra_data_size(max);
        self
    }

    /// Returns the chain spec used by the validator.
    #[inline]
    fn chain_spec(&self) -> &ChainSpec {
        self.inner.chain_spec()
    }
}

impl<ChainSpec, Types> PayloadValidator<Types> for EthereumEngineValidator<ChainSpec>
where
    ChainSpec: EthChainSpec + EthereumHardforks + 'static,
    Types: PayloadTypes<ExecutionData = ExecutionData>,
{
    type Block = Block;

    fn convert_payload_to_block(
        &self,
        payload: ExecutionData,
    ) -> Result<SealedBlock<Self::Block>, NewPayloadError> {
        self.inner.ensure_well_formed_payload(payload).map_err(Into::into)
    }
}

impl<ChainSpec, Types> EngineApiValidator<Types> for EthereumEngineValidator<ChainSpec>
where
    ChainSpec: EthChainSpec + EthereumHardforks + 'static,
    Types: PayloadTypes<PayloadAttributes = EthPayloadAttributes, ExecutionData = ExecutionData>,
{
    fn validate_version_specific_fields(
        &self,
        version: EngineApiMessageVersion,
        payload_or_attrs: PayloadOrAttributes<'_, Types::ExecutionData, EthPayloadAttributes>,
    ) -> Result<(), EngineObjectValidationError> {
        payload_or_attrs
            .execution_requests()
            .map(|requests| validate_execution_requests(requests))
            .transpose()?;

        validate_version_specific_fields(self.chain_spec(), version, payload_or_attrs)
    }

    fn ensure_well_formed_attributes(
        &self,
        version: EngineApiMessageVersion,
        attributes: &EthPayloadAttributes,
    ) -> Result<(), EngineObjectValidationError> {
        validate_version_specific_fields(
            self.chain_spec(),
            version,
            PayloadOrAttributes::<Types::ExecutionData, EthPayloadAttributes>::PayloadAttributes(
                attributes,
            ),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloy_primitives::B256;
    use alloy_rpc_types_engine::PayloadError;
    use reth_chainspec::ChainSpecBuilder;
    use reth_ethereum_engine_primitives::EthEngineTypes;
    use reth_payload_primitives::PayloadTypes;
    use reth_primitives_traits::Block as _;

    fn payload(extra_data_size: usize) -> ExecutionData {
        let mut block = Block::default();
        block.header.base_fee_per_gas = Some(1);
        block.header.extra_data = vec![42; extra_data_size].into();
        <EthEngineTypes as PayloadTypes>::block_to_payload(block.seal_slow(), None)
    }

    #[test]
    fn chain_extra_data_limit_preserves_hash_and_fork_validation() {
        let spec = Arc::new(ChainSpecBuilder::mainnet().build());
        let ethereum = EthereumEngineValidator::new(spec);
        assert!(<EthereumEngineValidator as PayloadValidator<EthEngineTypes>>::convert_payload_to_block(
            &ethereum, payload(32)
        ).is_ok());
        assert!(matches!(
            <EthereumEngineValidator as PayloadValidator<EthEngineTypes>>::convert_payload_to_block(
                &ethereum,
                payload(33)
            ),
            Err(NewPayloadError::Eth(PayloadError::ExtraData(_)))
        ));
        let fluent = ethereum.with_max_extra_data_size(65);
        let fluent: &dyn PayloadValidator<EthEngineTypes, Block = Block> = &fluent;
        let original = payload(65);
        let expected_hash = original.block_hash();
        let block = fluent.convert_payload_to_block(original).unwrap();
        assert_eq!(block.header().extra_data.len(), 65);
        assert_eq!(block.hash(), expected_hash);
        assert!(matches!(
            fluent.convert_payload_to_block(payload(66)),
            Err(NewPayloadError::Eth(PayloadError::ExtraData(_)))
        ));
        let mut bad_hash = payload(65);
        bad_hash.payload.as_v1_mut().block_hash = B256::ZERO;
        assert!(matches!(
            fluent.convert_payload_to_block(bad_hash),
            Err(NewPayloadError::Eth(PayloadError::BlockHash { .. }))
        ));

        // A payload valid before Shanghai must still be rejected when withdrawals are required.
        let shanghai = EthereumEngineValidator::new(Arc::new(
            ChainSpecBuilder::mainnet().shanghai_activated().build(),
        ))
        .with_max_extra_data_size(65);
        assert!(<EthereumEngineValidator as PayloadValidator<EthEngineTypes>>::convert_payload_to_block(&shanghai, payload(65)).is_err());
    }
}
