//! Minimal protobuf wire-format writer (varint / zigzag / length-delimited).

/// Append a varint (base-128, LSB first).
pub fn uvarint(buf: &mut Vec<u8>, mut v: u64) {
    while v >= 0x80 {
        buf.push((v as u8 & 0x7f) | 0x80);
        v >>= 7;
    }
    buf.push(v as u8);
}

/// ZigZag-encode a signed i64 (so negative deltas pack well).
pub fn zigzag(v: i64) -> u64 {
    ((v << 1) ^ (v >> 63)) as u64
}

/// Append a tag + varint field.
pub fn varint_field(buf: &mut Vec<u8>, field: u32, value: u64) {
    uvarint(buf, (field << 3) as u64);
    uvarint(buf, value);
}

/// Append a tag + length-delimited (bytes/string/message) field.
pub fn bytes_field(buf: &mut Vec<u8>, field: u32, payload: &[u8]) {
    uvarint(buf, ((field << 3) | 2).into()); // wiretype 2: length-delimited
    uvarint(buf, payload.len() as u64);
    buf.extend_from_slice(payload);
}

/// Append a tag + fixed32 field (wiretype 5; e.g. MVT Value.float_value).
pub fn fixed32_field(buf: &mut Vec<u8>, field: u32, bits: u32) {
    uvarint(buf, ((field << 3) | 5).into());
    buf.extend_from_slice(&bits.to_le_bytes());
}

/// Append a tag + packed repeated varint field.
pub fn packed_varints(buf: &mut Vec<u8>, field: u32, values: &[u64]) {
    let mut payload = Vec::with_capacity(values.len() * 2);
    for v in values {
        uvarint(&mut payload, *v);
    }
    bytes_field(buf, field, &payload);
}
