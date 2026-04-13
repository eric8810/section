#![allow(dead_code)]
// TODO(#002): wire the delta prototype into a future incremental transport path.

use super::hash_bytes;
use anyhow::Result;
use opendal::{Operator, Reader};

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ChunkSignature {
    pub(crate) offset: u64,
    pub(crate) len: usize,
    pub(crate) weak: u32,
    pub(crate) strong: String,
}

#[derive(Debug, Clone)]
pub(crate) struct RollingHash {
    a: u32,
    b: u32,
    window_len: usize,
}

impl RollingHash {
    pub(crate) fn from_window(window: &[u8]) -> Self {
        let mut a = 0_u32;
        let mut b = 0_u32;
        for (idx, byte) in window.iter().enumerate() {
            let value = *byte as u32;
            a = a.wrapping_add(value);
            b = b.wrapping_add((window.len() - idx) as u32 * value);
        }
        Self {
            a,
            b,
            window_len: window.len(),
        }
    }

    pub(crate) fn value(&self) -> u32 {
        (self.b << 16) | (self.a & 0xffff)
    }

    pub(crate) fn roll(&mut self, outgoing: u8, incoming: u8) -> u32 {
        let outgoing = outgoing as u32;
        let incoming = incoming as u32;
        self.a = self.a.wrapping_sub(outgoing).wrapping_add(incoming);
        self.b = self
            .b
            .wrapping_sub((self.window_len as u32).wrapping_mul(outgoing))
            .wrapping_add(self.a);
        self.value()
    }
}

pub(crate) fn build_chunk_signatures(bytes: &[u8], chunk_size: usize) -> Vec<ChunkSignature> {
    if chunk_size == 0 {
        return Vec::new();
    }

    bytes
        .chunks(chunk_size)
        .enumerate()
        .map(|(index, chunk)| ChunkSignature {
            offset: (index * chunk_size) as u64,
            len: chunk.len(),
            weak: RollingHash::from_window(chunk).value(),
            strong: hash_bytes(chunk),
        })
        .collect()
}

pub(crate) fn build_remote_chunk_signatures(
    rt: &tokio::runtime::Runtime,
    operator: &Operator,
    path: &str,
    chunk_size: usize,
) -> Result<Vec<ChunkSignature>> {
    if chunk_size == 0 {
        return Ok(Vec::new());
    }

    let meta = rt.block_on(operator.stat(path))?;
    let total = meta.content_length();
    let reader = rt.block_on(operator.reader(path))?;
    read_remote_signatures(rt, reader, total, chunk_size)
}

fn read_remote_signatures(
    rt: &tokio::runtime::Runtime,
    reader: Reader,
    total: u64,
    chunk_size: usize,
) -> Result<Vec<ChunkSignature>> {
    let mut signatures = Vec::new();
    let mut offset = 0_u64;

    while offset < total {
        let end = (offset + chunk_size as u64).min(total);
        let chunk = rt.block_on(reader.read(offset..end))?;
        let bytes = chunk.to_bytes();
        signatures.push(ChunkSignature {
            offset,
            len: bytes.len(),
            weak: RollingHash::from_window(bytes.as_ref()).value(),
            strong: hash_bytes(bytes.as_ref()),
        });
        offset = end;
    }

    Ok(signatures)
}

#[cfg(test)]
mod tests {
    use super::{build_chunk_signatures, build_remote_chunk_signatures, RollingHash};
    use opendal::services;
    use opendal::Operator;
    use std::fs;

    fn fs_operator(root: &std::path::Path) -> Operator {
        let builder = services::Fs::default().root(root.to_str().expect("utf8 path"));
        Operator::new(builder).expect("operator").finish()
    }

    #[test]
    fn rolling_hash_updates_match_full_rehash() {
        let initial = b"abcd";
        let mut rolling = RollingHash::from_window(initial);
        let next_hash = rolling.roll(b'a', b'e');
        let recomputed = RollingHash::from_window(b"bcde").value();
        assert_eq!(next_hash, recomputed);
    }

    #[test]
    fn remote_range_reads_can_build_matching_chunk_signatures() {
        let remote = tempfile::tempdir().expect("remote tempdir");
        let data = b"abcdefghijklmnopqrstuvwxyz0123456789";
        fs::write(remote.path().join("blob.bin"), data).expect("write remote");
        let rt = tokio::runtime::Runtime::new().expect("runtime");

        let local = build_chunk_signatures(data, 8);
        let remote = build_remote_chunk_signatures(&rt, &fs_operator(remote.path()), "blob.bin", 8)
            .expect("remote chunk signatures");

        assert_eq!(remote, local);
    }
}
