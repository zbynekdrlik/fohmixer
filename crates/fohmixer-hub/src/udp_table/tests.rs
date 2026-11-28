use super::*;

#[test]
fn a_rows_port_is_read_in_network_byte_order() {
    // 41234 = 0xA112: the table holds the bytes A1 12 first.
    assert_eq!(port_of(u32::from_ne_bytes([0xA1, 0x12, 0, 0])), 41_234);
    assert_eq!(port_of(u32::from_ne_bytes([0x9C, 0x40, 0, 0])), 40_000);
    assert_eq!(
        port_of(u32::from_ne_bytes([0x9C, 0x40, 0xFF, 0xFF])),
        40_000,
        "the upper bytes are no part of the port"
    );
    assert_eq!(port_of(0), 0);
}
