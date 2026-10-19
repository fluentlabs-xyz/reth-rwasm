//! Validates execution payload wrt Ethereum consensus rules

use alloy_consensus::Block;
use alloy_rpc_types_engine::{ExecutionData, PayloadError};
use reth_chainspec::EthereumHardforks;
use reth_payload_validator::{amsterdam, cancun, prague, shanghai};
use reth_primitives_traits::{Block as _, SealedBlock, SignedTransaction};
use std::sync::Arc;

/// Execution payload validator.
#[derive(Clone, Debug)]
pub struct EthereumExecutionPayloadValidator<ChainSpec> {
    /// Chain spec to validate against.
    chain_spec: Arc<ChainSpec>,
    /// Maximum extra-data length accepted by this chain.
    max_extra_data_size: usize,
}

impl<ChainSpec> EthereumExecutionPayloadValidator<ChainSpec> {
    /// Create a new validator.
    pub const fn new(chain_spec: Arc<ChainSpec>) -> Self {
        Self {
            chain_spec,
            max_extra_data_size: alloy_consensus::constants::MAXIMUM_EXTRA_DATA_SIZE,
        }
    }

    /// Overrides the chain-specific extra-data limit. Ethereum defaults to 32 bytes.
    pub const fn with_max_extra_data_size(mut self, max: usize) -> Self {
        self.max_extra_data_size = max;
        self
    }

    /// Returns the chain spec used by the validator.
    #[inline]
    pub const fn chain_spec(&self) -> &Arc<ChainSpec> {
        &self.chain_spec
    }
}

impl<ChainSpec: EthereumHardforks> EthereumExecutionPayloadValidator<ChainSpec> {
    /// Ensures that the given payload does not violate any consensus rules that concern the block's
    /// layout,
    ///
    /// See also [`ensure_well_formed_payload`]
    pub fn ensure_well_formed_payload<T: SignedTransaction>(
        &self,
        payload: ExecutionData,
    ) -> Result<SealedBlock<Block<T>>, PayloadError> {
        ensure_well_formed_payload_with_extra_data_limit(
            &self.chain_spec,
            payload,
            self.max_extra_data_size,
        )
    }
}

/// Ensures that the given payload does not violate any consensus rules that concern the block's
/// layout, like:
///    - missing or invalid base fee
///    - invalid extra data
///    - invalid transactions
///    - incorrect hash
///    - the versioned hashes passed with the payload do not exactly match transaction versioned
///      hashes
///    - the block does not contain blob transactions if it is pre-cancun
///
/// The checks are done in the order that conforms with the engine-API specification.
///
/// This is intended to be invoked after receiving the payload from the CLI.
/// The additional [`MaybeCancunPayloadFields`](alloy_rpc_types_engine::MaybeCancunPayloadFields) are not part of the payload, but are additional fields in the `engine_newPayloadV3` RPC call, See also <https://github.com/ethereum/execution-apis/blob/fe8e13c288c592ec154ce25c534e26cb7ce0530d/src/engine/cancun.md#engine_newpayloadv3>
///
/// If the cancun fields are provided this also validates that the versioned hashes in the block
/// match the versioned hashes passed in the
/// [`CancunPayloadFields`](alloy_rpc_types_engine::CancunPayloadFields), if the cancun payload
/// fields are provided. If the payload fields are not provided, but versioned hashes exist
/// in the block, this is considered an error: [`PayloadError::InvalidVersionedHashes`].
///
/// This validates versioned hashes according to the Engine API Cancun spec:
/// <https://github.com/ethereum/execution-apis/blob/fe8e13c288c592ec154ce25c534e26cb7ce0530d/src/engine/cancun.md#specification>
pub fn ensure_well_formed_payload<ChainSpec, T>(
    chain_spec: ChainSpec,
    payload: ExecutionData,
) -> Result<SealedBlock<Block<T>>, PayloadError>
where
    ChainSpec: EthereumHardforks,
    T: SignedTransaction,
{
    ensure_well_formed_payload_with_extra_data_limit(
        chain_spec,
        payload,
        alloy_consensus::constants::MAXIMUM_EXTRA_DATA_SIZE,
    )
}

fn ensure_well_formed_payload_with_extra_data_limit<ChainSpec, T>(
    chain_spec: ChainSpec,
    payload: ExecutionData,
    max_extra_data_size: usize,
) -> Result<SealedBlock<Block<T>>, PayloadError>
where
    ChainSpec: EthereumHardforks,
    T: SignedTransaction,
{
    let ExecutionData { mut payload, sidecar } = payload;
    let expected_hash = payload.block_hash();
    let extra_data = std::mem::take(&mut payload.as_v1_mut().extra_data);
    if extra_data.len() > max_extra_data_size {
        return Err(PayloadError::ExtraData(extra_data))
    }

    // Alloy enforces Ethereum's fixed 32-byte limit while parsing. Apply the configured
    // chain limit here and restore the original bytes before hashing or fork validation.
    let mut block = payload.try_into_block_with_sidecar(&sidecar)?;
    block.header.extra_data = extra_data;
    let sealed_block = block.seal_slow();

    // Ensure the hash included in the payload matches the block hash
    if expected_hash != sealed_block.hash() {
        return Err(PayloadError::BlockHash {
            execution: sealed_block.hash(),
            consensus: expected_hash,
        })
    }

    shanghai::ensure_well_formed_fields(
        sealed_block.body(),
        chain_spec.is_shanghai_active_at_timestamp(sealed_block.timestamp),
    )?;

    cancun::ensure_well_formed_fields(
        &sealed_block,
        sidecar.cancun(),
        chain_spec.is_cancun_active_at_timestamp(sealed_block.timestamp),
    )?;

    prague::ensure_well_formed_fields(
        sealed_block.body(),
        sidecar.prague(),
        chain_spec.is_prague_active_at_timestamp(sealed_block.timestamp),
    )?;

    amsterdam::ensure_well_formed_fields(
        &sealed_block,
        chain_spec.is_amsterdam_active_at_timestamp(sealed_block.timestamp),
    )?;

    Ok(sealed_block)
}
