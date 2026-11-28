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

#[test]
fn a_tables_rows_are_its_counts_ports_and_pids() {
    let port = |p: u16| {
        let [high, low] = p.to_be_bytes();
        u32::from_ne_bytes([high, low, 0, 0])
    };
    let table = [
        2,
        0x0100_007F,
        port(41_234),
        10,
        0,
        port(45_678),
        20,
        0,
        port(40_000),
        30,
    ];
    assert_eq!(
        rows_of(&table),
        vec![(41_234, 10), (45_678, 20)],
        "the count's rows, never the words past it"
    );
    assert_eq!(rows_of(&table[..7]), vec![(41_234, 10), (45_678, 20)]);
    assert_eq!(
        rows_of(&table[..6]),
        vec![(41_234, 10)],
        "a cut row is none"
    );
    assert!(rows_of(&[]).is_empty());
    assert!(rows_of(&[0, 1, port(41_234), 10]).is_empty());
    assert_eq!(ROW_WORDS, 3);
}
