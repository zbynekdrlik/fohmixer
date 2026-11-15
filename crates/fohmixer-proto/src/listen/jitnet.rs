//! Jitter's networking chunks (#210): what `jit.net.send` writes to a TCP
//! socket, read by the hub, as the Max SDK's "Jitter Networking
//! Specification" lays them out.
//!
//! Every chunk starts with an 8-byte header, a four-character id and a
//! 32-bit size. `JMTX` is a matrix: a 288-byte header (`id`, `size` = 288,
//! `planecount`, `type` 0 char / 1 long / 2 float32 / 3 float64,
//! `dimcount`, 32 `dim`, 32 `dimstride` in bytes, `datasize`, a double
//! `time`), then `datasize` bytes of cells. `JMMP` is a message: its size,
//! its argument count, then atoms, each a type byte `s` (a NUL-ended
//! string), `l` (a 32-bit int) or `f` (a 32-bit float). The receiver answers
//! each matrix with a `JMLP` latency chunk ([`latency`]).
//!
//! **Byte order.** The spec's text says little-endian, its own example code
//! reads big-endian, and an open-source sender writes the header
//! big-endian (with its chunk size left little-endian). So every number is
//! read in the order its own field tells: a matrix header's order is the
//! one in which its `size` reads 288, a message's the one in which its size
//! is small ([`small`]); the samples are read in the matrix header's order.
//! Which order Max really writes is probe P3 on #210 (`.claude/rules/listen.md`).
//!
//! The hub takes float32 matrices of 1 or 2 planes (the channels) in 1 or 2
//! dimensions (`jit.catch~ @mode 1` gives `framesize` × k cells; a 0
//! dimension with no data is an empty matrix), at most
//! [`MAX_DATA`] bytes; anything else is a [`JitError`] and the connection
//! ends (its framing can no longer be trusted).
//!
//! Pure: no I/O; slices and iterators only, never a hand-kept index.

/// The chunk header: id and size.
pub const CHUNK_HEADER: usize = 8;
/// The matrix header (its own `id` and `size` included).
pub const MATRIX_HEADER: usize = 288;
/// The most matrix data one chunk may carry: 25 of `jit.catch~`'s
/// 960-sample frames of 2 float32 channels (25 × 960 × 2 × 4 = 192 000
/// bytes, 0.5 s of sound). Its `qmetro 10` takes one frame or two at a
/// time; 25 covers Live's low-priority queue held up for half a second,
/// and a matrix is never more than ~25 Opus frames of encoding on the
/// hub's async path.
pub const MAX_DATA: usize = 25 * 960 * 2 * 4;
/// The largest message chunk.
pub const MAX_MESSAGE: usize = 4096;
/// The most atoms one message may carry.
pub const MAX_ATOMS: usize = 64;
/// The dimensions a matrix header lists.
pub const MAX_DIMS: usize = 32;
/// The latency chunk's length: its id, 4 bytes of padding (the C struct's
/// alignment of the doubles), three doubles.
pub const LATENCY_LEN: usize = 32;
/// The `type` of a float32 matrix.
pub const FLOAT32: i32 = 2;
/// The matrix chunk's id.
pub const MATRIX_ID: [u8; 4] = *b"JMTX";
/// The message chunk's id.
pub const MESSAGE_ID: [u8; 4] = *b"JMMP";
/// The latency chunk's id.
pub const LATENCY_ID: [u8; 4] = *b"JMLP";

/// A byte order.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Order {
    Big,
    Little,
}

impl Order {
    /// A 32-bit int from four bytes.
    pub fn i32(self, b: [u8; 4]) -> i32 {
        match self {
            Order::Big => i32::from_be_bytes(b),
            Order::Little => i32::from_le_bytes(b),
        }
    }

    /// A float32 from four bytes.
    pub fn f32(self, b: [u8; 4]) -> f32 {
        match self {
            Order::Big => f32::from_be_bytes(b),
            Order::Little => f32::from_le_bytes(b),
        }
    }

    /// A double from eight bytes.
    pub fn f64(self, b: [u8; 8]) -> f64 {
        match self {
            Order::Big => f64::from_be_bytes(b),
            Order::Little => f64::from_le_bytes(b),
        }
    }

    fn i32_bytes(self, v: i32) -> [u8; 4] {
        match self {
            Order::Big => v.to_be_bytes(),
            Order::Little => v.to_le_bytes(),
        }
    }

    fn f32_bytes(self, v: f32) -> [u8; 4] {
        match self {
            Order::Big => v.to_be_bytes(),
            Order::Little => v.to_le_bytes(),
        }
    }

    fn f64_bytes(self, v: f64) -> [u8; 8] {
        match self {
            Order::Big => v.to_be_bytes(),
            Order::Little => v.to_le_bytes(),
        }
    }

    /// A four-character id as this order writes the C multi-character
    /// constant: as spelled in big-endian, reversed in little-endian.
    fn id(self, id: [u8; 4]) -> [u8; 4] {
        match self {
            Order::Big => id,
            Order::Little => [id[3], id[2], id[1], id[0]],
        }
    }
}

/// One atom of a message.
#[derive(Debug, Clone, PartialEq)]
pub enum Atom {
    Sym(String),
    Long(i32),
    Float(f32),
}

/// A matrix's cells: `samples` holds `planes` values per cell, cell after
/// cell (dim 0 fastest), as interleaved audio.
#[derive(Debug, Clone, PartialEq)]
pub struct Matrix {
    pub order: Order,
    pub planes: usize,
    pub cells: usize,
    pub samples: Vec<f32>,
    /// The sender's time (ms), for the latency answer.
    pub time: f64,
}

/// One chunk read.
#[derive(Debug, Clone, PartialEq)]
pub enum Chunk {
    Matrix(Matrix),
    Message(Vec<Atom>),
}

/// Why a stream was refused.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum JitError {
    /// An unknown chunk id.
    Id,
    /// A size field that is no size of this chunk.
    Size,
    /// Not float32.
    Type,
    /// Not 1 or 2 planes.
    Planes,
    /// Not 1 or 2 dimensions, or one of them empty or too large.
    Dims,
    /// Strides that do not fit the cells or the data.
    Strides,
    /// More than [`MAX_DATA`] bytes.
    DataSize,
    /// A message that does not parse.
    Message,
}

impl JitError {
    /// Its word for the hub's counters.
    pub fn reason(self) -> &'static str {
        match self {
            JitError::Id => "chunk_id",
            JitError::Size => "chunk_size",
            JitError::Type => "matrix_type",
            JitError::Planes => "matrix_planes",
            JitError::Dims => "matrix_dims",
            JitError::Strides => "matrix_strides",
            JitError::DataSize => "matrix_data",
            JitError::Message => "message",
        }
    }
}

/// Four bytes at `at` of `b` (none past its end).
fn four(b: &[u8], at: usize) -> Option<[u8; 4]> {
    b.get(at..at.checked_add(4)?)?.try_into().ok()
}

/// Eight bytes at `at` of `b`.
fn eight(b: &[u8], at: usize) -> Option<[u8; 8]> {
    b.get(at..at.checked_add(8)?)?.try_into().ok()
}

/// A size field's value and order: the order in which it reads within
/// `1..=max` (big-endian first); none when neither does.
pub fn small(b: [u8; 4], max: usize) -> Option<(usize, Order)> {
    [Order::Big, Order::Little].into_iter().find_map(|order| {
        let v = usize::try_from(order.i32(b)).ok()?;
        (1..=max).contains(&v).then_some((v, order))
    })
}

/// What a chunk header announces.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Kind {
    Matrix,
    Message,
}

/// A chunk id in either order.
fn kind_of(id: [u8; 4]) -> Result<Kind, JitError> {
    [Order::Big, Order::Little]
        .into_iter()
        .find_map(|order| {
            if id == order.id(MATRIX_ID) {
                Some(Kind::Matrix)
            } else if id == order.id(MESSAGE_ID) {
                Some(Kind::Message)
            } else {
                None
            }
        })
        .ok_or(JitError::Id)
}

/// A matrix header's layout, checked.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Layout {
    order: Order,
    planes: usize,
    cols: usize,
    rows: usize,
    col_stride: usize,
    row_stride: usize,
    datasize: usize,
}

/// A non-negative header field.
fn field(order: Order, h: &[u8], at: usize, err: JitError) -> Result<usize, JitError> {
    let raw = four(h, at).ok_or(err)?;
    usize::try_from(order.i32(raw)).map_err(|_| err)
}

/// Reads and checks a matrix header (`h`: its 288 bytes).
fn layout(h: &[u8]) -> Result<(Layout, f64), JitError> {
    let id = four(h, 0).ok_or(JitError::Size)?;
    if kind_of(id)? != Kind::Matrix {
        return Err(JitError::Id);
    }
    let size = four(h, 4).ok_or(JitError::Size)?;
    let order = [Order::Big, Order::Little]
        .into_iter()
        .find(|o| usize::try_from(o.i32(size)) == Ok(MATRIX_HEADER))
        .ok_or(JitError::Size)?;
    let planes = field(order, h, 8, JitError::Planes)?;
    let kind = order.i32(four(h, 12).ok_or(JitError::Type)?);
    let dimcount = field(order, h, 16, JitError::Dims)?;
    if kind != FLOAT32 {
        return Err(JitError::Type);
    }
    if !(1..=2).contains(&planes) {
        return Err(JitError::Planes);
    }
    if !(1..=2).contains(&dimcount) {
        return Err(JitError::Dims);
    }
    let dim = |i: usize| field(order, h, 20 + 4 * i, JitError::Dims);
    let stride = |i: usize| field(order, h, 20 + 4 * MAX_DIMS + 4 * i, JitError::Strides);
    let cols = dim(0)?;
    let rows = if dimcount == 2 { dim(1)? } else { 1 };
    let cell = 4 * planes;
    let col_stride = stride(0)?;
    let row_stride = if dimcount == 2 { stride(1)? } else { 0 };
    let datasize = field(order, h, 20 + 8 * MAX_DIMS, JitError::DataSize)?;
    let time = order.f64(eight(h, 24 + 8 * MAX_DIMS).ok_or(JitError::Size)?);
    if cols.checked_mul(rows).is_none_or(|n| n > MAX_DATA / 4) {
        return Err(JitError::Dims);
    }
    if datasize > MAX_DATA {
        return Err(JitError::DataSize);
    }
    if cols == 0 || rows == 0 {
        // No cell: an empty matrix (`jit.catch~` with less than a frame
        // caught), consumed and answered; data behind it is inconsistent.
        if datasize != 0 {
            return Err(JitError::Strides);
        }
        let layout = Layout {
            order,
            planes,
            cols,
            rows,
            col_stride,
            row_stride,
            datasize,
        };
        return Ok((layout, time));
    }
    if col_stride < cell
        || (rows > 1 && cols.checked_mul(col_stride).is_none_or(|n| row_stride < n))
    {
        return Err(JitError::Strides);
    }
    // The last cell's end, within the data.
    let extent = (rows - 1)
        .checked_mul(row_stride)
        .and_then(|r| r.checked_add((cols - 1).checked_mul(col_stride)?))
        .and_then(|e| e.checked_add(cell));
    if extent.is_none_or(|e| e > datasize) {
        return Err(JitError::Strides);
    }
    let layout = Layout {
        order,
        planes,
        cols,
        rows,
        col_stride,
        row_stride,
        datasize,
    };
    Ok((layout, time))
}

/// The cells of `data` as interleaved samples (`layout` checked: every
/// read is within `data`).
fn samples(l: &Layout, data: &[u8]) -> Vec<f32> {
    (0..l.rows)
        .flat_map(|r| (0..l.cols).map(move |c| r * l.row_stride + c * l.col_stride))
        .flat_map(|cell| (0..l.planes).map(move |p| cell + 4 * p))
        .map(|at| four(data, at).map_or(0.0, |b| l.order.f32(b)))
        .collect()
}

/// A message chunk's atoms (`payload`: its `size` bytes).
fn message(payload: &[u8]) -> Result<Vec<Atom>, JitError> {
    let (size, order) = four(payload, 0)
        .and_then(|b| small(b, payload.len()))
        .ok_or(JitError::Message)?;
    let body = payload.get(8..size).ok_or(JitError::Message)?;
    atoms(body, order)
}

/// The atoms of a message body (trailing NULs are padding).
fn atoms(body: &[u8], order: Order) -> Result<Vec<Atom>, JitError> {
    let mut out = Vec::new();
    let mut rest = body;
    for _ in 0..=MAX_ATOMS {
        if rest.iter().all(|&b| b == 0) {
            return Ok(out);
        }
        let (&tag, tail) = rest.split_first().ok_or(JitError::Message)?;
        rest = match tag {
            b's' => {
                let len = tail.iter().position(|&b| b == 0).ok_or(JitError::Message)?;
                let (text, after) = tail.split_at(len);
                let text = std::str::from_utf8(text).map_err(|_| JitError::Message)?;
                out.push(Atom::Sym(text.to_string()));
                after.get(1..).ok_or(JitError::Message)?
            }
            b'l' | b'f' => {
                let b = four(tail, 0).ok_or(JitError::Message)?;
                out.push(if tag == b'l' {
                    Atom::Long(order.i32(b))
                } else {
                    Atom::Float(order.f32(b))
                });
                tail.get(4..).ok_or(JitError::Message)?
            }
            _ => return Err(JitError::Message),
        };
    }
    Err(JitError::Message)
}

/// A stream's reader: bytes in ([`Reader::push`]), chunks out
/// ([`Reader::next_chunk`]). It buffers at most one chunk (at most
/// [`CHUNK_HEADER`] + [`MATRIX_HEADER`] + [`MAX_DATA`] bytes) plus what
/// was pushed after it.
#[derive(Debug, Default)]
pub struct Reader {
    buf: Vec<u8>,
}

impl Reader {
    pub fn push(&mut self, bytes: &[u8]) {
        self.buf.extend_from_slice(bytes);
    }

    /// The bytes waiting for the rest of their chunk.
    pub fn buffered(&self) -> usize {
        self.buf.len()
    }

    /// The next whole chunk, none until its bytes are all in, or the error
    /// that ends the stream.
    pub fn next_chunk(&mut self) -> Result<Option<Chunk>, JitError> {
        let Some(id) = four(&self.buf, 0) else {
            return Ok(None);
        };
        let Some(size) = four(&self.buf, 4) else {
            return Ok(None);
        };
        match kind_of(id)? {
            Kind::Matrix => {
                small(size, MATRIX_HEADER)
                    .filter(|(n, _)| *n == MATRIX_HEADER)
                    .ok_or(JitError::Size)?;
                let Some(h) = self.buf.get(CHUNK_HEADER..CHUNK_HEADER + MATRIX_HEADER) else {
                    return Ok(None);
                };
                let (l, time) = layout(h)?;
                let start = CHUNK_HEADER + MATRIX_HEADER;
                let Some(data) = self.buf.get(start..start + l.datasize) else {
                    return Ok(None);
                };
                let matrix = Matrix {
                    order: l.order,
                    planes: l.planes,
                    cells: l.cols * l.rows,
                    samples: samples(&l, data),
                    time,
                };
                self.buf.drain(..start + l.datasize);
                Ok(Some(Chunk::Matrix(matrix)))
            }
            Kind::Message => {
                let (len, _) = small(size, MAX_MESSAGE).ok_or(JitError::Size)?;
                let Some(payload) = self.buf.get(CHUNK_HEADER..CHUNK_HEADER + len) else {
                    return Ok(None);
                };
                let atoms = message(payload)?;
                self.buf.drain(..CHUNK_HEADER + len);
                Ok(Some(Chunk::Message(atoms)))
            }
        }
    }
}

/// The latency chunk answering a matrix sent at `client` (ms), read at
/// `before` and used at `after` (the hub's ms), in the matrix's order.
pub fn latency(order: Order, client: f64, before: f64, after: f64) -> [u8; LATENCY_LEN] {
    let mut out = [0u8; LATENCY_LEN];
    out[..4].copy_from_slice(&order.id(LATENCY_ID));
    out[8..16].copy_from_slice(&order.f64_bytes(client));
    out[16..24].copy_from_slice(&order.f64_bytes(before));
    out[24..32].copy_from_slice(&order.f64_bytes(after));
    out
}

/// A matrix chunk of float32 `samples` (`planes` per cell, `cols` × `rows`
/// cells, rows packed), in `order`: the tests' sender.
pub fn encode_matrix(
    order: Order,
    planes: usize,
    cols: usize,
    rows: usize,
    samples: &[f32],
    time: f64,
) -> Vec<u8> {
    let cell = 4 * planes;
    let data_len = samples.len() * 4;
    let as_i32 = |n: usize| i32::try_from(n).unwrap_or(i32::MAX);
    let mut out = Vec::with_capacity(CHUNK_HEADER + MATRIX_HEADER + data_len);
    out.extend_from_slice(&order.id(MATRIX_ID));
    out.extend_from_slice(&order.i32_bytes(as_i32(MATRIX_HEADER)));
    out.extend_from_slice(&order.id(MATRIX_ID));
    out.extend_from_slice(&order.i32_bytes(as_i32(MATRIX_HEADER)));
    out.extend_from_slice(&order.i32_bytes(as_i32(planes)));
    out.extend_from_slice(&order.i32_bytes(FLOAT32));
    let dims: &[usize] = if rows > 1 { &[cols, rows] } else { &[cols] };
    out.extend_from_slice(&order.i32_bytes(as_i32(dims.len())));
    let strides = [cell, cell * cols];
    for slot in 0..MAX_DIMS {
        let v = dims.get(slot).copied().unwrap_or(0);
        out.extend_from_slice(&order.i32_bytes(as_i32(v)));
    }
    for slot in 0..MAX_DIMS {
        let v = if slot < dims.len() {
            strides.get(slot).copied().unwrap_or(0)
        } else {
            0
        };
        out.extend_from_slice(&order.i32_bytes(as_i32(v)));
    }
    out.extend_from_slice(&order.i32_bytes(as_i32(data_len)));
    out.extend_from_slice(&order.f64_bytes(time));
    for s in samples {
        out.extend_from_slice(&order.f32_bytes(*s));
    }
    out
}

/// A message chunk of `atoms` in `order`: the tests' sender.
pub fn encode_message(order: Order, atoms: &[Atom]) -> Vec<u8> {
    let mut body = Vec::new();
    for atom in atoms {
        match atom {
            Atom::Sym(s) => {
                body.push(b's');
                body.extend_from_slice(s.as_bytes());
                body.push(0);
            }
            Atom::Long(n) => {
                body.push(b'l');
                body.extend_from_slice(&order.i32_bytes(*n));
            }
            Atom::Float(f) => {
                body.push(b'f');
                body.extend_from_slice(&order.f32_bytes(*f));
            }
        }
    }
    let size = i32::try_from(body.len() + 8).unwrap_or(i32::MAX);
    let argc = i32::try_from(atoms.len()).unwrap_or(i32::MAX);
    let mut out = Vec::with_capacity(body.len() + 16);
    out.extend_from_slice(&order.id(MESSAGE_ID));
    out.extend_from_slice(&order.i32_bytes(size));
    out.extend_from_slice(&order.i32_bytes(size));
    out.extend_from_slice(&order.i32_bytes(argc));
    out.extend_from_slice(&body);
    out
}

#[cfg(test)]
mod tests;
