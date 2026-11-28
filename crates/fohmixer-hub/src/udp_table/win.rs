//! The OS's IPv4 UDP endpoint table with each endpoint's process (#264):
//! `GetExtendedUdpTable` with `UDP_TABLE_OWNER_PID`. Windows only, excluded
//! from mutation testing (not compiled on Linux CI); its decisions are the
//! tested `udp_table::rows_of` and `poll_ports::shared_by_processes`.

use std::io;

use windows::Win32::Foundation::ERROR_INSUFFICIENT_BUFFER;
use windows::Win32::NetworkManagement::IpHelper::{
    GetExtendedUdpTable, MIB_UDPROW_OWNER_PID, UDP_TABLE_OWNER_PID,
};
use windows::Win32::Networking::WinSock::AF_INET;

use super::{ROW_WORDS, rows_of};
use crate::events::EventLog;
use crate::lufs_udp::{Polls, os_moves};

/// One read of the table over the devices' `range` and its moves
/// (`lufs_udp::os_moves`, at once: the LUFS listener's 30 s tick).
pub fn check(
    poller: &std::net::UdpSocket,
    polls: &mut Polls,
    range: &std::ops::Range<u16>,
    events: &EventLog,
) {
    os_moves(
        poller,
        polls,
        rows(),
        range,
        std::time::Instant::now(),
        events,
    );
}

/// `rows_of` reads a row as `ROW_WORDS` words.
const _: () = assert!(size_of::<MIB_UDPROW_OWNER_PID>() == ROW_WORDS * size_of::<u32>());

/// Reads of a table that grew between the size's answer and the read.
const TRIES: usize = 4;

/// Every IPv4 UDP endpoint's port and process.
pub fn rows() -> io::Result<Vec<(u16, u32)>> {
    let mut size: u32 = 0;
    for _ in 0..TRIES {
        let words = usize::try_from(size).unwrap_or(usize::MAX).div_ceil(4);
        let mut table = vec![0u32; words];
        let into = (!table.is_empty()).then(|| table.as_mut_ptr().cast());
        // SAFETY: `into` is none or points at `table`, `size` bytes or more
        // of 4-byte aligned words; the call writes at most `size` bytes and
        // sets `size` to the bytes it needs when they are more.
        let code = unsafe {
            GetExtendedUdpTable(
                into,
                &mut size,
                false,
                u32::from(AF_INET.0),
                UDP_TABLE_OWNER_PID,
                0,
            )
        };
        match code {
            0 => return Ok(rows_of(&table)),
            code if code == ERROR_INSUFFICIENT_BUFFER.0 => continue,
            code => return Err(io::Error::from_raw_os_error(code as i32)),
        }
    }
    Err(io::Error::other("the UDP table grew at every read"))
}
