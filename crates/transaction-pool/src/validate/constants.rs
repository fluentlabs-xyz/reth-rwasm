/// [`TX_SLOT_BYTE_SIZE`] is used to calculate how many data slots a single transaction
/// takes up based on its byte size. The slots are used as `DoS` protection, ensuring
/// that validating a new transaction remains a constant operation (in reality
/// O(maxslots), where the default maximum is 32 slots).
pub const TX_SLOT_BYTE_SIZE: usize = 32 * 1024;

/// [`DEFAULT_MAX_TX_INPUT_BYTES`] is the default maximum size a single transaction can have. This
/// field has non-trivial consequences: larger transactions are significantly harder and
/// more expensive to propagate; larger transactions also take more resources
/// to validate whether they fit into the pool or not. Fluent allows 32 slots of 32 KiB,
/// for a default maximum of 1 MiB.
pub const DEFAULT_MAX_TX_INPUT_BYTES: usize = 32 * TX_SLOT_BYTE_SIZE;
