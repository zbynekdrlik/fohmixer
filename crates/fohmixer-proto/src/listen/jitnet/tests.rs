use super::*;

fn ramp(n: usize) -> Vec<f32> {
    (0..n).map(|i| i as f32 / 1024.0 - 0.5).collect()
}

fn read_all(reader: &mut Reader) -> Vec<Chunk> {
    let mut out = Vec::new();
    for _ in 0..64 {
        match reader.next_chunk() {
            Ok(Some(chunk)) => out.push(chunk),
            Ok(None) => return out,
            Err(e) => panic!("{e:?}"),
        }
    }
    panic!("more than 64 chunks")
}

#[test]
fn a_two_plane_two_dim_matrix_reads_in_either_order() {
    for order in [Order::Big, Order::Little] {
        let samples = ramp(2 * 960 * 3);
        let bytes = encode_matrix(order, 2, 960, 3, &samples, 1234.5);
        assert_eq!(
            bytes.len(),
            CHUNK_HEADER + MATRIX_HEADER + samples.len() * 4
        );
        let mut reader = Reader::default();
        reader.push(&bytes);
        let Some(Chunk::Matrix(m)) = reader.next_chunk().unwrap() else {
            panic!("a matrix");
        };
        assert_eq!(m.order, order);
        assert_eq!((m.planes, m.cells), (2, 2880));
        assert_eq!(m.samples, samples);
        assert_eq!(m.time, 1234.5);
        assert_eq!(reader.buffered(), 0);
        assert_eq!(reader.next_chunk(), Ok(None));
    }
}

#[test]
fn the_ids_are_the_c_constants_in_each_order() {
    let big = encode_matrix(Order::Big, 1, 4, 1, &[0.0; 4], 0.0);
    assert_eq!(&big[..4], b"JMTX");
    assert_eq!(&big[4..8], &[0, 0, 1, 32]);
    let little = encode_matrix(Order::Little, 1, 4, 1, &[0.0; 4], 0.0);
    assert_eq!(&little[..4], b"XTMJ");
    assert_eq!(&little[4..8], &[32, 1, 0, 0]);
    let msg = encode_message(Order::Big, &[Atom::Long(1)]);
    assert_eq!(&msg[..4], b"JMMP");
    assert_eq!(&encode_message(Order::Little, &[])[..4], b"PMMJ");
}

#[test]
fn a_one_dim_mono_matrix_reads() {
    let samples = ramp(500);
    let mut reader = Reader::default();
    reader.push(&encode_matrix(Order::Big, 1, 500, 1, &samples, 0.0));
    let Some(Chunk::Matrix(m)) = reader.next_chunk().unwrap() else {
        panic!("a matrix");
    };
    assert_eq!((m.planes, m.cells), (1, 500));
    assert_eq!(m.samples, samples);
}

#[test]
fn a_chunk_split_anywhere_waits_for_its_bytes() {
    let samples = ramp(2 * 960);
    let mut bytes = encode_matrix(Order::Big, 2, 960, 1, &samples, 7.0);
    bytes.extend(encode_message(
        Order::Big,
        &[Atom::Sym("x".into()), Atom::Long(5)],
    ));
    for cut in [1, 4, 7, 8, 100, 295, 296, 297, bytes.len() - 1] {
        let mut reader = Reader::default();
        reader.push(&bytes[..cut]);
        let first = read_all(&mut reader);
        reader.push(&bytes[cut..]);
        let mut all = first;
        all.extend(read_all(&mut reader));
        assert_eq!(all.len(), 2, "cut at {cut}");
        assert!(matches!(&all[0], Chunk::Matrix(m) if m.samples == samples));
        assert_eq!(
            all[1],
            Chunk::Message(vec![Atom::Sym("x".into()), Atom::Long(5)])
        );
        assert_eq!(reader.buffered(), 0);
    }
}

#[test]
fn padded_strides_skip_the_padding() {
    // Two rows of three mono cells, each row padded to 16 bytes.
    let mut bytes = encode_matrix(Order::Big, 1, 3, 2, &[0.0; 6], 0.0);
    let stride_at = CHUNK_HEADER + 20 + 4 * MAX_DIMS + 4;
    bytes[stride_at..stride_at + 4].copy_from_slice(&16i32.to_be_bytes());
    let datasize_at = CHUNK_HEADER + 20 + 8 * MAX_DIMS;
    bytes[datasize_at..datasize_at + 4].copy_from_slice(&32i32.to_be_bytes());
    bytes.truncate(CHUNK_HEADER + MATRIX_HEADER);
    let rows: [[f32; 4]; 2] = [[1.0, 2.0, 3.0, 99.0], [4.0, 5.0, 6.0, 99.0]];
    for v in rows.iter().flatten() {
        bytes.extend_from_slice(&v.to_be_bytes());
    }
    let mut reader = Reader::default();
    reader.push(&bytes);
    let Some(Chunk::Matrix(m)) = reader.next_chunk().unwrap() else {
        panic!("a matrix");
    };
    assert_eq!(m.samples, vec![1.0, 2.0, 3.0, 4.0, 5.0, 6.0]);
}

/// The matrix header field at `field` (bytes into the 288) set to `v`.
fn with_field(order: Order, field: usize, v: i32) -> Vec<u8> {
    let mut bytes = encode_matrix(order, 2, 4, 2, &[0.0; 16], 0.0);
    let at = CHUNK_HEADER + field;
    let b = match order {
        Order::Big => v.to_be_bytes(),
        Order::Little => v.to_le_bytes(),
    };
    bytes[at..at + 4].copy_from_slice(&b);
    bytes
}

fn error_of(bytes: &[u8]) -> JitError {
    let mut reader = Reader::default();
    reader.push(bytes);
    reader.next_chunk().unwrap_err()
}

const PLANES: usize = 8;
const TYPE: usize = 12;
const DIMCOUNT: usize = 16;
const DIM0: usize = 20;
const DIM1: usize = 24;
const STRIDE0: usize = 20 + 4 * MAX_DIMS;
const STRIDE1: usize = STRIDE0 + 4;
const DATASIZE: usize = 20 + 8 * MAX_DIMS;

#[test]
fn a_bad_matrix_header_is_refused_with_its_reason() {
    for order in [Order::Big, Order::Little] {
        assert_eq!(error_of(&with_field(order, TYPE, 0)), JitError::Type);
        assert_eq!(error_of(&with_field(order, TYPE, 3)), JitError::Type);
        assert_eq!(error_of(&with_field(order, PLANES, 0)), JitError::Planes);
        assert_eq!(error_of(&with_field(order, PLANES, 3)), JitError::Planes);
        assert_eq!(error_of(&with_field(order, PLANES, -1)), JitError::Planes);
        assert_eq!(error_of(&with_field(order, DIMCOUNT, 0)), JitError::Dims);
        assert_eq!(error_of(&with_field(order, DIMCOUNT, 3)), JitError::Dims);
        assert_eq!(error_of(&with_field(order, DIM0, 0)), JitError::Dims);
        assert_eq!(error_of(&with_field(order, DIM1, 0)), JitError::Dims);
        assert_eq!(error_of(&with_field(order, DIM0, i32::MAX)), JitError::Dims);
        assert_eq!(error_of(&with_field(order, STRIDE0, 7)), JitError::Strides);
        assert_eq!(error_of(&with_field(order, STRIDE1, 31)), JitError::Strides);
        assert_eq!(
            error_of(&with_field(order, DATASIZE, 63)),
            JitError::Strides
        );
        assert_eq!(
            error_of(&with_field(order, DATASIZE, (MAX_DATA + 1) as i32)),
            JitError::DataSize
        );
        assert_eq!(error_of(&with_field(order, 4, 287)), JitError::Size);
        assert_eq!(error_of(&with_field(order, 0, 0x4142_4344)), JitError::Id);
    }
    // A matrix header whose own id is a message's.
    assert_eq!(
        error_of(&with_field(Order::Big, 0, i32::from_be_bytes(*b"JMMP"))),
        JitError::Id
    );
}

#[test]
fn the_edges_of_a_matrix_header_are_taken() {
    // Exactly MAX_DATA bytes, one row: taken.
    let cols = MAX_DATA / 8;
    let samples = vec![0.25f32; cols * 2];
    let mut reader = Reader::default();
    reader.push(&encode_matrix(Order::Big, 2, cols, 1, &samples, 0.0));
    let Some(Chunk::Matrix(m)) = reader.next_chunk().unwrap() else {
        panic!("a matrix");
    };
    assert_eq!(m.cells, cols);
    // The last cell ending exactly at datasize: taken; one byte short: not.
    let exact = with_field(Order::Big, DATASIZE, 64);
    let mut reader = Reader::default();
    reader.push(&exact);
    assert!(matches!(reader.next_chunk(), Ok(Some(Chunk::Matrix(_)))));
    // A stride of exactly one cell, and a row stride of exactly a row.
    let mut reader = Reader::default();
    reader.push(&with_field(Order::Big, STRIDE0, 8));
    assert!(matches!(reader.next_chunk(), Ok(Some(Chunk::Matrix(_)))));
    let mut reader = Reader::default();
    reader.push(&with_field(Order::Big, STRIDE1, 32));
    assert!(matches!(reader.next_chunk(), Ok(Some(Chunk::Matrix(_)))));
}

#[test]
fn a_one_row_two_dim_matrix_ignores_its_row_stride() {
    let mut bytes = encode_matrix(Order::Big, 2, 4, 2, &[0.5; 16], 0.0);
    let at = CHUNK_HEADER + DIM1;
    bytes[at..at + 4].copy_from_slice(&1i32.to_be_bytes());
    let st = CHUNK_HEADER + STRIDE1;
    bytes[st..st + 4].copy_from_slice(&0i32.to_be_bytes());
    let ds = CHUNK_HEADER + DATASIZE;
    bytes[ds..ds + 4].copy_from_slice(&32i32.to_be_bytes());
    bytes.truncate(CHUNK_HEADER + MATRIX_HEADER + 32);
    let mut reader = Reader::default();
    reader.push(&bytes);
    let Some(Chunk::Matrix(m)) = reader.next_chunk().unwrap() else {
        panic!("a matrix");
    };
    assert_eq!(m.cells, 4);
    assert_eq!(m.samples, vec![0.5; 8]);
}

#[test]
fn a_bad_chunk_header_is_refused() {
    let mut matrix = encode_matrix(Order::Big, 1, 4, 1, &[0.0; 4], 0.0);
    matrix[7] = 33;
    assert_eq!(error_of(&matrix), JitError::Size);
    assert_eq!(error_of(b"ABCD\0\0\0\x08"), JitError::Id);
    // Little-endian chunk size of a big-endian id (an open-source sender).
    let mut mixed = encode_matrix(Order::Big, 1, 4, 1, &[1.0; 4], 0.0);
    mixed[4..8].copy_from_slice(&288i32.to_le_bytes());
    let mut reader = Reader::default();
    reader.push(&mixed);
    assert!(matches!(reader.next_chunk(), Ok(Some(Chunk::Matrix(m))) if m.samples == [1.0; 4]));
    let mut message = encode_message(Order::Big, &[Atom::Long(1)]);
    message[4..8].copy_from_slice(&((MAX_MESSAGE + 1) as i32).to_be_bytes());
    assert_eq!(error_of(&message), JitError::Size);
    message[4..8].copy_from_slice(&0i32.to_be_bytes());
    assert_eq!(error_of(&message), JitError::Size);
}

#[test]
fn a_message_reads_its_atoms_in_either_order() {
    let atoms = vec![
        Atom::Sym("fohmixer_listen".into()),
        Atom::Long(-3),
        Atom::Float(48_000.0),
        Atom::Sym(String::new()),
    ];
    for order in [Order::Big, Order::Little] {
        let mut reader = Reader::default();
        reader.push(&encode_message(order, &atoms));
        assert_eq!(reader.next_chunk(), Ok(Some(Chunk::Message(atoms.clone()))));
        assert_eq!(reader.buffered(), 0);
    }
}

#[test]
fn a_message_with_padding_reads_and_a_bad_one_is_refused() {
    let mut padded = encode_message(Order::Big, &[Atom::Long(1)]);
    padded.extend_from_slice(&[0, 0, 0]);
    let len = (padded.len() - CHUNK_HEADER) as i32;
    padded[4..8].copy_from_slice(&len.to_be_bytes());
    let mut reader = Reader::default();
    reader.push(&padded);
    assert_eq!(
        reader.next_chunk(),
        Ok(Some(Chunk::Message(vec![Atom::Long(1)])))
    );
    let bad = |body: &[u8]| {
        let mut bytes = b"JMMP".to_vec();
        let n = (body.len() + 8) as i32;
        bytes.extend_from_slice(&n.to_be_bytes());
        bytes.extend_from_slice(&n.to_be_bytes());
        bytes.extend_from_slice(&1i32.to_be_bytes());
        bytes.extend_from_slice(body);
        error_of(&bytes)
    };
    assert_eq!(bad(b"x"), JitError::Message);
    assert_eq!(bad(b"sabc"), JitError::Message);
    assert_eq!(bad(b"l\0\0"), JitError::Message);
    assert_eq!(bad(b"s\xff\0"), JitError::Message);
    let many: Vec<u8> = std::iter::repeat_n(*b"l\0\0\0\x01", MAX_ATOMS + 1)
        .flatten()
        .collect();
    assert_eq!(bad(&many), JitError::Message);
    let enough: Vec<u8> = std::iter::repeat_n(*b"l\0\0\0\x01", MAX_ATOMS)
        .flatten()
        .collect();
    let mut bytes = b"JMMP".to_vec();
    let n = (enough.len() + 8) as i32;
    bytes.extend_from_slice(&n.to_be_bytes());
    bytes.extend_from_slice(&n.to_be_bytes());
    bytes.extend_from_slice(&0i32.to_be_bytes());
    bytes.extend_from_slice(&enough);
    let mut reader = Reader::default();
    reader.push(&bytes);
    assert!(matches!(reader.next_chunk(), Ok(Some(Chunk::Message(a))) if a.len() == MAX_ATOMS));
    // An inner size under its own 8 bytes.
    let mut short = encode_message(Order::Big, &[Atom::Long(1)]);
    short[8..12].copy_from_slice(&4i32.to_be_bytes());
    assert_eq!(error_of(&short), JitError::Message);
}

#[test]
fn small_reads_a_size_in_the_order_it_fits() {
    assert_eq!(small(40i32.to_be_bytes(), 100), Some((40, Order::Big)));
    assert_eq!(small(40i32.to_le_bytes(), 100), Some((40, Order::Little)));
    assert_eq!(small(100i32.to_be_bytes(), 100), Some((100, Order::Big)));
    assert_eq!(small(101i32.to_be_bytes(), 100), None);
    assert_eq!(small(1i32.to_be_bytes(), 100), Some((1, Order::Big)));
    assert_eq!(small(0i32.to_be_bytes(), 100), None);
    assert_eq!(small((-1i32).to_be_bytes(), 100), None);
}

#[test]
fn the_latency_answer_is_laid_out_as_the_c_struct() {
    let big = latency(Order::Big, 1.5, 2.5, 3.5);
    assert_eq!(&big[..4], b"JMLP");
    assert_eq!(&big[4..8], &[0; 4]);
    assert_eq!(&big[8..16], &1.5f64.to_be_bytes());
    assert_eq!(&big[16..24], &2.5f64.to_be_bytes());
    assert_eq!(&big[24..32], &3.5f64.to_be_bytes());
    let little = latency(Order::Little, 1.5, 2.5, 3.5);
    assert_eq!(&little[..4], b"PLMJ");
    assert_eq!(&little[8..16], &1.5f64.to_le_bytes());
    assert_eq!(&little[24..32], &3.5f64.to_le_bytes());
}

#[test]
fn errors_name_their_reason() {
    let all = [
        (JitError::Id, "chunk_id"),
        (JitError::Size, "chunk_size"),
        (JitError::Type, "matrix_type"),
        (JitError::Planes, "matrix_planes"),
        (JitError::Dims, "matrix_dims"),
        (JitError::Strides, "matrix_strides"),
        (JitError::DataSize, "matrix_data"),
        (JitError::Message, "message"),
    ];
    for (e, word) in all {
        assert_eq!(e.reason(), word);
    }
    assert_eq!(MAX_DATA, 1_048_576);
    assert_eq!(MAX_MESSAGE, 4096);
    assert_eq!(MAX_ATOMS, 64);
}
