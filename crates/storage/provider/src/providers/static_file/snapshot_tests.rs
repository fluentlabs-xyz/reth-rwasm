//! Committed-snapshot reader tests: a reader of the shared jar cache observes only what a writer
//! committed, and a torn on-disk snapshot surfaces as a typed error instead of a torn read.

use super::{StaticFileProvider, StaticFileProviderBuilder, StaticFileWriter};
use crate::HeaderProvider;
use alloy_consensus::Header;
use alloy_primitives::{Address, BlockHash, U256};
use reth_db::{models::AccountBeforeTx, test_utils::create_test_static_files_dir};
use reth_ethereum_primitives::EthPrimitives;
use reth_primitives_traits::Account;
use reth_static_file_types::{find_fixed_range, StaticFileSegment};
use std::fs;

/// Writes `blocks` committed `AccountChangeSets` blocks (5 changes each).
fn write_account_changesets(sf_rw: &StaticFileProvider<EthPrimitives>, blocks: u64) {
    let mut writer = sf_rw.latest_writer(StaticFileSegment::AccountChangeSets).unwrap();
    for block_num in 0..blocks {
        let changeset: Vec<AccountBeforeTx> = (0..5u8)
            .map(|i| {
                let mut addr = Address::ZERO;
                addr.0[0] = block_num as u8;
                addr.0[1] = i;
                AccountBeforeTx {
                    address: addr,
                    info: Some(Account {
                        nonce: block_num,
                        balance: U256::from(block_num),
                        ..Default::default()
                    }),
                }
            })
            .collect();
        writer.append_account_changeset(changeset, block_num).unwrap();
    }
    writer.commit().unwrap();
}

/// Builds a read-write provider over `static_dir` with `blocks_per_file` blocks per file.
fn provider(
    static_dir: &tempfile::TempDir,
    blocks_per_file: u64,
) -> StaticFileProvider<EthPrimitives> {
    StaticFileProviderBuilder::read_write(static_dir)
        .with_blocks_per_file(blocks_per_file)
        .build()
        .unwrap()
}

/// A writer-idle reader that self-loads a file whose changeset sidecar was truncated below the
/// header's committed length gets a typed, retryable `InconsistentSnapshot` error instead of a
/// cached torn view.
#[test]
fn test_torn_sidecar_self_load_errors_not_torn() {
    let (static_dir, _) = create_test_static_files_dir();
    let range = find_fixed_range(9, 10);
    let sidecar_path = static_dir
        .as_ref()
        .join(StaticFileSegment::AccountChangeSets.filename(&range))
        .with_extension("csoff");

    write_account_changesets(&provider(&static_dir, 10), 10);

    let file = fs::OpenOptions::new().write(true).open(&sidecar_path).unwrap();
    file.set_len(7 * 16).unwrap();
    file.sync_all().unwrap();

    let sf = provider(&static_dir, 10);
    sf.initialize_index().unwrap();

    let err = sf
        .get_segment_provider_for_block(StaticFileSegment::AccountChangeSets, 8, None)
        .expect_err("torn sidecar must surface an error, never a torn read");
    assert!(
        err.to_string().contains("inconsistent static-file snapshot"),
        "expected InconsistentSnapshot, got: {err}"
    );
}

/// An offsets file that no longer covers the committed rows — as a reader racing a mid-append
/// writer can observe — degrades to a typed error rather than a torn read.
#[test]
fn test_torn_offsets_self_load_errors_not_torn() {
    let (static_dir, _) = create_test_static_files_dir();
    let range = find_fixed_range(9, 10);
    let offsets_path =
        static_dir.as_ref().join(StaticFileSegment::Headers.filename(&range)).with_extension("off");

    {
        let sf_rw = provider(&static_dir, 10);
        let mut w = sf_rw.latest_writer(StaticFileSegment::Headers).unwrap();
        let mut header = Header::default();
        for num in 0..10 {
            header.number = num;
            w.append_header(&header, &BlockHash::default()).unwrap();
        }
        w.commit().unwrap();
    }

    // Keep only the offset-size byte: the table no longer covers the committed rows.
    let file = fs::OpenOptions::new().write(true).open(&offsets_path).unwrap();
    file.set_len(1).unwrap();
    file.sync_all().unwrap();

    let sf = provider(&static_dir, 10);
    sf.initialize_index().unwrap();

    let err = sf
        .get_segment_provider_for_block(StaticFileSegment::Headers, 5, None)
        .expect_err("torn offsets must surface an error, never a torn read");
    assert!(
        err.to_string().contains("inconsistent static-file snapshot"),
        "expected InconsistentSnapshot, got: {err}"
    );
}

/// A writer-idle, cleanly committed file still self-loads on a cache miss: the snapshot
/// validation does not break read-only or cross-process reads.
#[test]
fn test_writer_idle_self_load_still_works() {
    let (static_dir, _) = create_test_static_files_dir();
    write_account_changesets(&provider(&static_dir, 10), 10);

    let sf = provider(&static_dir, 10);
    sf.initialize_index().unwrap();

    let provider = sf
        .get_segment_provider_for_block(StaticFileSegment::AccountChangeSets, 8, None)
        .expect("clean idle file must self-load");
    let offsets = provider.read_changeset_offsets().unwrap();
    assert_eq!(offsets.unwrap().len(), 10);
}

/// While a writer is active on a file, reads observe only the committed snapshot: committed rows
/// are visible, the uncommitted tail is a clean `Ok(None)`.
#[test]
fn test_publish_on_open_committed_snapshot() {
    let (static_dir, _) = create_test_static_files_dir();
    let sf_rw = provider(&static_dir, 100);

    let mut header = Header::default();
    let mut w = sf_rw.latest_writer(StaticFileSegment::Headers).unwrap();
    for num in 0..5 {
        header.number = num;
        w.append_header(&header, &BlockHash::default()).unwrap();
    }
    w.commit().unwrap();
    for num in 5..10 {
        header.number = num;
        w.append_header(&header, &BlockHash::default()).unwrap();
    }

    assert!(sf_rw.header_by_number(4).unwrap().is_some(), "committed row visible");
    assert!(sf_rw.header_by_number(8).unwrap().is_none(), "uncommitted tail invisible");
}

/// After the writer rotates onto a new file, the rotated-past file stays a readable committed
/// snapshot and the new file's uncommitted rows are invisible.
#[test]
fn test_rotation_publishes_committed_snapshot() {
    let (static_dir, _) = create_test_static_files_dir();
    let sf_rw = provider(&static_dir, 5);

    let mut header = Header::default();
    let mut w = sf_rw.latest_writer(StaticFileSegment::Headers).unwrap();
    for num in 0..5 {
        header.number = num;
        w.append_header(&header, &BlockHash::default()).unwrap();
    }
    w.commit().unwrap();
    for num in 5..8 {
        header.number = num;
        w.append_header(&header, &BlockHash::default()).unwrap();
    }

    assert!(sf_rw.header_by_number(3).unwrap().is_some(), "rotated-past file readable");
    assert!(sf_rw.header_by_number(6).unwrap().is_none(), "uncommitted rotated tail invisible");
}

/// `initialize_index` clears the jar cache while the Headers writer stays registered (genesis
/// init, `delete_jar`, pruning). The next lookup of block 0 misses on the active file and must
/// still serve the committed snapshot: a refused read there wedges the engine, since a failing
/// read is what keeps the next commit from republishing.
#[test]
fn test_initialize_index_preserves_active_writer_reads() {
    let (static_dir, _) = create_test_static_files_dir();
    let sf_rw: StaticFileProvider<EthPrimitives> =
        StaticFileProviderBuilder::read_write(&static_dir).build().unwrap();

    {
        let mut w = sf_rw.latest_writer(StaticFileSegment::Headers).unwrap();
        let header = Header { number: 0, ..Default::default() };
        w.append_header(&header, &BlockHash::default()).unwrap();
        w.commit().unwrap();
    }

    sf_rw.initialize_index().unwrap();

    assert_eq!(
        sf_rw.header_by_number(0).expect("no snapshot error").map(|h| h.number),
        Some(0),
        "genesis header readable after re-init with active writer"
    );

    {
        let mut w = sf_rw.latest_writer(StaticFileSegment::Headers).unwrap();
        for num in 1..=3u64 {
            let header = Header { number: num, ..Default::default() };
            w.append_header(&header, &BlockHash::default()).unwrap();
        }
        w.commit().unwrap();
    }
    for num in 0..=3u64 {
        assert_eq!(
            sf_rw.header_by_number(num).unwrap().map(|h| h.number),
            Some(num),
            "block {num} readable, chain advanced past re-init"
        );
    }
}
