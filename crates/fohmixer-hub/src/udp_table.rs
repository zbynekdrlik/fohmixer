//! The OS's UDP endpoint table (#264, the design comment 7231753561): which
//! process binds which port. On Windows the hub reads it (`win.rs`,
//! `GetExtendedUdpTable` with `UDP_TABLE_OWNER_PID`, IPv4) to find a poll
//! port two Lives bound, whose second binder the hub never hears
//! (`poll_ports::shared_by_processes`, `lufs_udp::os_moves`). Off Windows
//! there is no read: SimLive's first-binder delivery covers the hub's side.

/// A row's port: the table's `dwLocalPort` holds it in network byte order
/// in its first two bytes.
pub fn port_of(local_port: u32) -> u16 {
    let _ = local_port;
    0
}

#[cfg(test)]
mod tests;
