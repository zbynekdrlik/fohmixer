//! The OS's UDP endpoint table (#264, the design comment 7231753561): which
//! process binds which port. On Windows the hub reads it (`win.rs`,
//! `GetExtendedUdpTable` with `UDP_TABLE_OWNER_PID`, IPv4) to find a poll
//! port two Lives bound, whose second binder the hub never hears
//! (`poll_ports::shared_by_processes`, `lufs_udp::os_moves`). Off Windows
//! there is no read: SimLive's first-binder delivery covers the hub's side.

#[cfg(windows)]
mod win;
#[cfg(windows)]
pub use win::{check, rows};

/// A row's port: the table's `dwLocalPort` holds it in network byte order
/// in its first two bytes.
pub fn port_of(local_port: u32) -> u16 {
    let [high, low, ..] = local_port.to_ne_bytes();
    u16::from_be_bytes([high, low])
}

/// The rows of a `MIB_UDPTABLE_OWNER_PID` read into `table` (4-byte words:
/// `dwNumEntries`, then each `MIB_UDPROW_OWNER_PID`'s `dwLocalAddr`,
/// `dwLocalPort` and `dwOwningPid`): each row's port and pid, at most the
/// count's and the words' rows.
pub fn rows_of(table: &[u32]) -> Vec<(u16, u32)> {
    let Some((&count, rest)) = table.split_first() else {
        return Vec::new();
    };
    rest.as_chunks::<ROW_WORDS>()
        .0
        .iter()
        .take(usize::try_from(count).unwrap_or(usize::MAX))
        .map(|row| (port_of(row[1]), row[2]))
        .collect()
}

/// The words of one `MIB_UDPROW_OWNER_PID`.
pub const ROW_WORDS: usize = 3;

#[cfg(test)]
mod tests;
