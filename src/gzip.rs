//! Unpacking a `.gz` file as a stream (farm board WO-10). The model files from 3D Print Rig are gzipped
//! GLB, 10 to 150 MB once unpacked; they are checked against their published SHA-256 as they unpack, so
//! nothing larger than a 64 KiB window and one output chunk is ever held. Decode only, no compression,
//! no new crate. The decoder is the plain canonical-Huffman method from RFC 1951.
use std::io::Read;

type Result<T> = std::result::Result<T, String>;

const CUT_SHORT: &str = "the file is cut short";
const DAMAGED: &str = "the file is damaged";

struct Huffman {
    count: [u16; 16],
    symbol: Vec<u16>,
}

impl Huffman {
    fn new(lengths: &[u8]) -> Result<Self> {
        let mut count = [0_u16; 16];
        for &l in lengths {
            count[l as usize] += 1;
        }
        let mut left = 1_i32;
        for len in 1..16 {
            left = (left << 1) - i32::from(count[len]);
            if left < 0 {
                return Err(DAMAGED.into());
            }
        }
        let mut offs = [0_u16; 16];
        for len in 1..15 {
            offs[len + 1] = offs[len] + count[len];
        }
        let mut symbol = vec![0_u16; lengths.len()];
        for (s, &l) in lengths.iter().enumerate() {
            if l != 0 {
                symbol[offs[l as usize] as usize] = s as u16;
                offs[l as usize] += 1;
            }
        }
        Ok(Self { count, symbol })
    }
}

const LEN_BASE: [u16; 29] = [
    3, 4, 5, 6, 7, 8, 9, 10, 11, 13, 15, 17, 19, 23, 27, 31, 35, 43, 51, 59, 67, 83, 99, 115, 131,
    163, 195, 227, 258,
];
const LEN_EXTRA: [u8; 29] = [
    0, 0, 0, 0, 0, 0, 0, 0, 1, 1, 1, 1, 2, 2, 2, 2, 3, 3, 3, 3, 4, 4, 4, 4, 5, 5, 5, 5, 0,
];
const DIST_BASE: [u16; 30] = [
    1, 2, 3, 4, 5, 7, 9, 13, 17, 25, 33, 49, 65, 97, 129, 193, 257, 385, 513, 769, 1025, 1537,
    2049, 3073, 4097, 6145, 8193, 12289, 16385, 24577,
];
const DIST_EXTRA: [u8; 30] = [
    0, 0, 0, 0, 1, 1, 2, 2, 3, 3, 4, 4, 5, 5, 6, 6, 7, 7, 8, 8, 9, 9, 10, 10, 11, 11, 12, 12, 13,
    13,
];
const ORDER: [usize; 19] = [
    16, 17, 18, 0, 8, 7, 9, 6, 10, 5, 11, 4, 12, 3, 13, 2, 14, 1, 15,
];

const WINDOW: usize = 32768;
const CHUNK: usize = 64 * 1024;

struct Inflater<'a, R: Read, F: FnMut(&[u8]) -> Result<()>> {
    src: R,
    inbuf: Vec<u8>,
    inpos: usize,
    inlen: usize,
    bitbuf: u32,
    bitcnt: u32,
    window: Vec<u8>,
    wpos: usize,
    out: Vec<u8>,
    total: u64,
    crc: u32,
    sink: &'a mut F,
}

impl<R: Read, F: FnMut(&[u8]) -> Result<()>> Inflater<'_, R, F> {
    fn byte(&mut self) -> Result<u8> {
        if self.inpos == self.inlen {
            self.inlen = self
                .src
                .read(&mut self.inbuf)
                .map_err(|e| format!("cannot read the file: {e}"))?;
            self.inpos = 0;
            if self.inlen == 0 {
                return Err(CUT_SHORT.into());
            }
        }
        let b = self.inbuf[self.inpos];
        self.inpos += 1;
        Ok(b)
    }

    fn bits(&mut self, need: u32) -> Result<u32> {
        while self.bitcnt < need {
            let b = u32::from(self.byte()?);
            self.bitbuf |= b << self.bitcnt;
            self.bitcnt += 8;
        }
        let v = self.bitbuf & ((1_u32 << need) - 1);
        self.bitbuf >>= need;
        self.bitcnt -= need;
        Ok(v)
    }

    fn decode(&mut self, h: &Huffman) -> Result<u16> {
        let (mut code, mut first, mut index) = (0_i32, 0_i32, 0_i32);
        for len in 1..16 {
            code |= self.bits(1)? as i32;
            let count = i32::from(h.count[len]);
            if code - count < first {
                return Ok(h.symbol[(index + (code - first)) as usize]);
            }
            index += count;
            first = (first + count) << 1;
            code <<= 1;
        }
        Err(DAMAGED.into())
    }

    fn put(&mut self, b: u8) -> Result<()> {
        self.window[self.wpos] = b;
        self.wpos = (self.wpos + 1) % WINDOW;
        self.out.push(b);
        self.total += 1;
        if self.out.len() >= CHUNK {
            self.flush()?;
        }
        Ok(())
    }

    fn flush(&mut self) -> Result<()> {
        if !self.out.is_empty() {
            self.crc = crc32(self.crc, &self.out);
            (self.sink)(&self.out)?;
            self.out.clear();
        }
        Ok(())
    }

    fn codes(&mut self, lit: &Huffman, dist: &Huffman) -> Result<()> {
        loop {
            let sym = self.decode(lit)? as usize;
            match sym {
                0..=255 => self.put(sym as u8)?,
                256 => return Ok(()),
                257..=285 => {
                    let i = sym - 257;
                    let len = LEN_BASE[i] as usize + self.bits(u32::from(LEN_EXTRA[i]))? as usize;
                    let d = self.decode(dist)? as usize;
                    if d >= 30 {
                        return Err(DAMAGED.into());
                    }
                    let distance =
                        DIST_BASE[d] as usize + self.bits(u32::from(DIST_EXTRA[d]))? as usize;
                    if distance as u64 > self.total {
                        return Err(DAMAGED.into());
                    }
                    for _ in 0..len {
                        let b = self.window[(self.wpos + WINDOW - distance) % WINDOW];
                        self.put(b)?;
                    }
                }
                _ => return Err(DAMAGED.into()),
            }
        }
    }

    fn block(&mut self) -> Result<bool> {
        let last = self.bits(1)? == 1;
        match self.bits(2)? {
            0 => {
                self.bitbuf = 0;
                self.bitcnt = 0;
                let (a, b, c, d) = (self.byte()?, self.byte()?, self.byte()?, self.byte()?);
                let len = u16::from_le_bytes([a, b]);
                if len != !u16::from_le_bytes([c, d]) {
                    return Err(DAMAGED.into());
                }
                for _ in 0..len {
                    let v = self.byte()?;
                    self.put(v)?;
                }
            }
            1 => {
                let mut l = [8_u8; 288];
                l[144..256].fill(9);
                l[256..280].fill(7);
                let lit = Huffman::new(&l)?;
                let dist = Huffman::new(&[5_u8; 30])?;
                self.codes(&lit, &dist)?;
            }
            2 => {
                let nlen = self.bits(5)? as usize + 257;
                let ndist = self.bits(5)? as usize + 1;
                let ncode = self.bits(4)? as usize + 4;
                if nlen > 286 || ndist > 30 {
                    return Err(DAMAGED.into());
                }
                let mut lengths = [0_u8; 19];
                for &o in ORDER.iter().take(ncode) {
                    lengths[o] = self.bits(3)? as u8;
                }
                let code = Huffman::new(&lengths)?;
                let mut all = vec![0_u8; nlen + ndist];
                let mut i = 0;
                while i < nlen + ndist {
                    let sym = self.decode(&code)?;
                    if sym < 16 {
                        all[i] = sym as u8;
                        i += 1;
                    } else {
                        let (value, rep) = match sym {
                            16 => {
                                if i == 0 {
                                    return Err(DAMAGED.into());
                                }
                                (all[i - 1], 3 + self.bits(2)? as usize)
                            }
                            17 => (0, 3 + self.bits(3)? as usize),
                            _ => (0, 11 + self.bits(7)? as usize),
                        };
                        if i + rep > nlen + ndist {
                            return Err(DAMAGED.into());
                        }
                        all[i..i + rep].fill(value);
                        i += rep;
                    }
                }
                if all[256] == 0 {
                    return Err(DAMAGED.into());
                }
                let lit = Huffman::new(&all[..nlen])?;
                let dist = Huffman::new(&all[nlen..])?;
                self.codes(&lit, &dist)?;
            }
            _ => return Err(DAMAGED.into()),
        }
        Ok(last)
    }
}

fn crc32(prev: u32, data: &[u8]) -> u32 {
    static TABLE: std::sync::OnceLock<[u32; 256]> = std::sync::OnceLock::new();
    let t = TABLE.get_or_init(|| {
        let mut t = [0_u32; 256];
        for (n, slot) in t.iter_mut().enumerate() {
            let mut c = n as u32;
            for _ in 0..8 {
                c = if c & 1 == 1 {
                    0xedb8_8320 ^ (c >> 1)
                } else {
                    c >> 1
                };
            }
            *slot = c;
        }
        t
    });
    let mut c = !prev;
    for &b in data {
        c = t[((c ^ u32::from(b)) & 0xff) as usize] ^ (c >> 8);
    }
    !c
}

/// Unpack a gzip stream, handing the bytes to `sink` in pieces. Returns the unpacked length. The CRC and
/// the length at the end of the file are checked, so a cut-short or altered file is an error.
pub fn gunzip<R: Read>(src: R, mut sink: impl FnMut(&[u8]) -> Result<()>) -> Result<u64> {
    let mut z = Inflater {
        src,
        inbuf: vec![0; 64 * 1024],
        inpos: 0,
        inlen: 0,
        bitbuf: 0,
        bitcnt: 0,
        window: vec![0; WINDOW],
        wpos: 0,
        out: Vec::with_capacity(CHUNK + 300),
        total: 0,
        crc: 0,
        sink: &mut sink,
    };
    let head = [z.byte()?, z.byte()?, z.byte()?, z.byte()?];
    if head[0] != 0x1f || head[1] != 0x8b || head[2] != 8 {
        return Err("this is not a gzip file".into());
    }
    for _ in 0..6 {
        z.byte()?; // modification time, extra flags, operating system
    }
    let flags = head[3];
    if flags & 4 != 0 {
        let n = u16::from_le_bytes([z.byte()?, z.byte()?]);
        for _ in 0..n {
            z.byte()?;
        }
    }
    for bit in [8, 16] {
        if flags & bit != 0 {
            while z.byte()? != 0 {}
        }
    }
    if flags & 2 != 0 {
        z.byte()?;
        z.byte()?;
    }
    while !z.block()? {}
    z.flush()?;
    // The trailer starts on a byte boundary: drop the bits left in the last byte.
    z.bitbuf = 0;
    z.bitcnt = 0;
    let want_crc = u32::from_le_bytes([z.byte()?, z.byte()?, z.byte()?, z.byte()?]);
    let want_len = u32::from_le_bytes([z.byte()?, z.byte()?, z.byte()?, z.byte()?]);
    if want_crc != z.crc || want_len != (z.total & 0xffff_ffff) as u32 {
        return Err(format!("{DAMAGED}: its checksum does not match"));
    }
    Ok(z.total)
}

#[cfg(test)]
mod tests {
    use super::*;

    // Made with a real gzip: a fixed-Huffman block with back-references, and a dynamic-Huffman block
    // (the 6456-byte text `line {i}: the quick brown fox {i*i%97}` for i in 0..200).
    const FIXED: &str = "1f8b08000000000002ffcb48cdc9c957c84027b9000088590b18000000";
    const DYNAMIC: &str = "1f8b08000000000002ff85995d6e55310c84df59c559429c3fdb2c07544445d5aa1508968fd0b5cdcb99c9b3efc9cfc4b1e7cb7d797e7dbadae7ebe7f7a7ebfdd7f3d71fd7978fb7dfafd7b7b73f57fbf4f22f2af7517944fb7d743ea2e33eea8fe80423ef4778dd87fb7a84f77d78c4d70a161673db7d78c7c2fd3e6cb16b01928d0803cd7a8c2e48358d38d04d7bc481721906ca8d5c3d906ee7f7403b0fe505883772fb403d8d8cea403d89f93b906fc5f81dc8e771f41dc83743fe0ee4f3cc69a0dfccf9817e9ee303fd56ee1fe8671106f2ed907f00f97a64cf00f2598c3f807c2b2f2d90afc7f10c209fc5d51a40be1df20e7473737f403ec9ef917c71fc03e917f109f49ba1cf04fa8dd8ff04fa497e0ff4cbe951f645764d209f853c13c8a75955817c9adf03f934e7771e5f8d8fbf84af6f75bebf35b83e6b5279d7e2a7b3363fdda53c3b96f1ec5aceb373379edd5bf8edd89ddfae3df8eddc93dfeebd7875d89b5797adbc3a6de3d56d3b2d8eda786d55e1b5593bafed3a786fd0c97b8b2ede9b74f3dea6ca7ba31aefadeabc375be3bddd847b03ebdc5bd8a0d6c4267736b6b833b2cd9d9529356666dcd7991f6c61e3aed2859b52efdcd3fae096d82735d4bea81bf74dadbc2be50037feb5d3b9a535ba7469c2b72eeda09db4c1c59736f9e9495bfcf8a5ed1317f0fc93660732687e4003841ec506103e327eb8c182e8a3f000e147f1010490e2a3cdab98c8a10c0a62905553382fc482286416a4096f05d20fbd441088784d31793713842296f1cdbba9f4433b16082335857343209047f2ac2090e4bd1b074f230849d214096212a91116b76582a864d70f0ec65010978cd2c1b935154426b9044426698d651ebcb5203649732e884eb446581c0f04f249fd404f53d869917ed8266294120a414a2abd0e8c278852eab011a654ba2050a98443a45229bb0ea02c8855eada2058a98b8768a5ae2ec295bafcfbf0da2010585228482c59c210b2541184cc92f1c3838d2068a9428ea8a55a01e2966a26085caa1de9e1d54b10ba544b44ec524d15c14bb565442fd5d8f5f47488f8e5ffdba31fec0922981c00114c19243bbdbe4286c9a34014932a208a299b8830a68ca61d9eaf05914c995d8432659711cb94e14630934b70fef62f1066323ef9ff0e8866eafb7d985f0febb7930058c2bf3937c9d338190000";

    fn unhex(s: &str) -> Vec<u8> {
        (0..s.len() / 2)
            .map(|i| u8::from_str_radix(&s[i * 2..i * 2 + 2], 16).unwrap())
            .collect()
    }

    fn run(bytes: &[u8]) -> Result<Vec<u8>> {
        let mut out = Vec::new();
        gunzip(bytes, |c| {
            out.extend_from_slice(c);
            Ok(())
        })?;
        Ok(out)
    }

    #[test]
    fn stored_block_round_trips() {
        // A stored (uncompressed) deflate block: "abc".
        let mut f = vec![
            0x1f, 0x8b, 8, 0, 0, 0, 0, 0, 0, 3, 1, 3, 0, 0xfc, 0xff, b'a', b'b', b'c',
        ];
        f.extend_from_slice(&crc32(0, b"abc").to_le_bytes());
        f.extend_from_slice(&3_u32.to_le_bytes());
        assert_eq!(run(&f).unwrap(), b"abc");
    }

    #[test]
    fn real_gzip_files_unpack_exactly() {
        assert_eq!(run(&unhex(FIXED)).unwrap(), b"hello hello hello hello\n");
        let want: String = (0..200)
            .map(|i| format!("line {i}: the quick brown fox {}\n", i * i % 97))
            .collect();
        let got = run(&unhex(DYNAMIC)).unwrap();
        assert_eq!(got, want.as_bytes());
        // Fed one byte at a time, which is how a slow network hands it over.
        struct OneByte<'a>(&'a [u8]);
        impl Read for OneByte<'_> {
            fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
                match self.0.split_first() {
                    Some((b, rest)) => {
                        buf[0] = *b;
                        self.0 = rest;
                        Ok(1)
                    }
                    None => Ok(0),
                }
            }
        }
        let bytes = unhex(DYNAMIC);
        let mut out = Vec::new();
        let n = gunzip(OneByte(&bytes), |c| {
            out.extend_from_slice(c);
            Ok(())
        })
        .unwrap();
        assert_eq!((n as usize, out), (want.len(), want.into_bytes()));
    }

    #[test]
    fn crc_matches_the_standard_check_value() {
        assert_eq!(crc32(0, b"123456789"), 0xcbf4_3926);
    }

    #[test]
    fn a_cut_short_or_altered_file_is_refused_in_words() {
        assert!(run(b"not gzip at all")
            .unwrap_err()
            .contains("not a gzip file"));
        let mut f = vec![
            0x1f, 0x8b, 8, 0, 0, 0, 0, 0, 0, 3, 1, 3, 0, 0xfc, 0xff, b'a', b'b', b'c',
        ];
        f.extend_from_slice(&crc32(0, b"abc").to_le_bytes());
        f.extend_from_slice(&3_u32.to_le_bytes());
        assert!(run(&f[..f.len() - 3]).unwrap_err().contains("cut short"));
        let n = f.len();
        f[n - 6] ^= 1; // a changed checksum
        assert!(run(&f).unwrap_err().contains("checksum"));
    }
}
