use super::*;
use reth_payload_primitives::BuiltPayloadExecutedBlock;
use reth_trie::{updates::TrieUpdates, HashedPostState};

/// A same-height SIBLING of the current canonical block arriving via
/// `InsertExecutedBlock` must be inserted, not skipped as "outdated": an external
/// consensus (Fluent `DPoS` deferred execution) imports exactly this shape to reorg
/// a speculatively executed block onto its finalized fork. A height-only skip gate
/// (`number <= canonical_block_number()`) drops it, leaving the follow-up FCU
/// permanently SYNCING on the unknown head hash.
#[tokio::test]
async fn test_engine_tree_insert_executed_same_height_sibling_inserted() {
    let mut test_harness = TestHarness::new(MAINNET.clone());

    let blocks: Vec<_> = test_harness.block_builder.get_executed_blocks(0..3).collect();
    test_harness = test_harness.with_blocks(blocks.clone());

    let tip = blocks.last().unwrap().recovered_block();
    let sibling =
        test_harness.block_builder.get_executed_block_with_number(tip.number, tip.parent_hash());
    let sibling_hash = sibling.recovered_block().hash();
    assert_ne!(sibling_hash, tip.hash());

    let payload = BuiltPayloadExecutedBlock {
        recovered_block: sibling.recovered_block,
        execution_output: sibling.execution_output,
        hashed_state: Arc::new(HashedPostState::default()),
        trie_updates: Arc::new(TrieUpdates::default()),
    };
    let _ = test_harness
        .tree
        .on_engine_message(FromEngine::Request(EngineApiRequest::InsertExecutedBlock(payload)))
        .unwrap();

    assert!(
        test_harness.tree.state.tree_state.contains_hash(&sibling_hash),
        "same-height sibling must land in the tree, not be skipped as outdated"
    );
    // The sibling is reorg-able: a follow-up FCU(head=sibling) can compute the
    // chain update instead of falling through to handle_missing_block (SYNCING).
    let chain_update = test_harness.tree.on_new_head(sibling_hash).unwrap();
    assert!(
        matches!(chain_update, Some(NewCanonicalChain::Reorg { .. })),
        "FCU to the inserted sibling resolves to a reorg, not a missing-block download"
    );
}
