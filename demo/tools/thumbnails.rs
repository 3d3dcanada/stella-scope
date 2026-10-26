// Adds PNG thumbnails to a sliced G-code file (polish board WO-02, 2026-10-18).
//
//   rustc -O --edition 2021 demo/tools/thumbnails.rs -o /tmp/thumbnails
//   /tmp/thumbnails IN.gcode OUT.gcode 16x16,313x173,440x240 part-a.stl:04D9C4 [part-b.stl:FF6A00 ...]
//
// PrusaSlicer's command line cannot draw the pictures it embeds (it needs its window), so a sliced demo file
// would carry none and the farm tiles and file cards would show nothing. This draws the part itself from its
// STL: a small z-buffer, an iso view from the front left, each STL in its own colour, a transparent
// background. The pictures go in as the usual `; thumbnail begin WxH BYTES` ... `; thumbnail end` comment
// blocks right after the file's first line; nothing else in the file is touched. The PNG is deflated by the
// system `gzip` (its header and trailer are stripped and the stream is wrapped as zlib).
use std::{
    fs,
    io::Write,
    process::{Command, Stdio},
};

type V = [f64; 3];

fn stl(path: &str) -> Vec<[V; 3]> {
    let b = fs::read(path).unwrap_or_else(|e| panic!("{path}: {e}"));
    let n = u32::from_le_bytes(b[80..84].try_into().unwrap()) as usize;
    assert_eq!(b.len(), 84 + 50 * n, "{path} is not a binary STL");
    let f = |o: usize| f32::from_le_bytes(b[o..o + 4].try_into().unwrap()) as f64;
    (0..n)
        .map(|i| {
            let o = 84 + 50 * i + 12;
            [
                [f(o), f(o + 4), f(o + 8)],
                [f(o + 12), f(o + 16), f(o + 20)],
                [f(o + 24), f(o + 28), f(o + 32)],
            ]
        })
        .collect()
}

fn sub(a: V, b: V) -> V {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}
fn dot(a: V, b: V) -> f64 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}
fn cross(a: V, b: V) -> V {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}
fn unit(a: V) -> V {
    let l = dot(a, a).sqrt().max(1e-12);
    [a[0] / l, a[1] / l, a[2] / l]
}

/// RGBA pixels, `w` by `h`, of the parts seen from the front left and above.
fn render(parts: &[(Vec<[V; 3]>, [f64; 3])], w: usize, h: usize) -> Vec<u8> {
    const SS: usize = 3; // supersampling
    let (az, el) = (38f64.to_radians(), 26f64.to_radians());
    let d = [az.sin() * el.cos(), -az.cos() * el.cos(), el.sin()]; // towards the camera
    let right = unit(cross([0.0, 0.0, 1.0], d));
    let up = cross(d, right);
    let (mut lo, mut hi) = ([f64::MAX; 2], [f64::MIN; 2]);
    for (tris, _) in parts {
        for t in tris {
            for p in t {
                let s = [dot(*p, right), dot(*p, up)];
                for k in 0..2 {
                    lo[k] = lo[k].min(s[k]);
                    hi[k] = hi[k].max(s[k]);
                }
            }
        }
    }
    let (sw, sh) = (w * SS, h * SS);
    let margin = 0.08;
    let scale = ((sw as f64 * (1.0 - 2.0 * margin)) / (hi[0] - lo[0]))
        .min((sh as f64 * (1.0 - 2.0 * margin)) / (hi[1] - lo[1]));
    let (cx, cy) = ((lo[0] + hi[0]) / 2.0, (lo[1] + hi[1]) / 2.0);
    let to_px = |p: V| {
        [
            sw as f64 / 2.0 + (dot(p, right) - cx) * scale,
            sh as f64 / 2.0 - (dot(p, up) - cy) * scale,
            dot(p, d),
        ]
    };
    let light = unit([-0.45, -0.55, 0.75]);
    let mut depth = vec![f64::MIN; sw * sh];
    let mut img = vec![[0u8; 4]; sw * sh];
    for (tris, col) in parts {
        for t in tris {
            let n = unit(cross(sub(t[1], t[0]), sub(t[2], t[0])));
            if dot(n, d) <= 0.0 {
                continue;
            }
            let shade = 0.28 + 0.72 * dot(n, light).max(0.0) + 0.12 * dot(n, d).max(0.0);
            let rgb = [col[0] * shade, col[1] * shade, col[2] * shade]
                .map(|c| (c * 255.0).clamp(0.0, 255.0) as u8);
            let (a, b, c) = (to_px(t[0]), to_px(t[1]), to_px(t[2]));
            let (x0, x1) = (
                (a[0].min(b[0]).min(c[0]).floor().max(0.0)) as usize,
                (a[0].max(b[0]).max(c[0]).ceil().min(sw as f64 - 1.0)) as usize,
            );
            let (y0, y1) = (
                (a[1].min(b[1]).min(c[1]).floor().max(0.0)) as usize,
                (a[1].max(b[1]).max(c[1]).ceil().min(sh as f64 - 1.0)) as usize,
            );
            let area = (b[0] - a[0]) * (c[1] - a[1]) - (b[1] - a[1]) * (c[0] - a[0]);
            if area.abs() < 1e-9 {
                continue;
            }
            for y in y0..=y1 {
                for x in x0..=x1 {
                    let (px, py) = (x as f64 + 0.5, y as f64 + 0.5);
                    let w0 = ((b[0] - px) * (c[1] - py) - (b[1] - py) * (c[0] - px)) / area;
                    let w1 = ((c[0] - px) * (a[1] - py) - (c[1] - py) * (a[0] - px)) / area;
                    let w2 = 1.0 - w0 - w1;
                    if w0 < 0.0 || w1 < 0.0 || w2 < 0.0 {
                        continue;
                    }
                    let z = w0 * a[2] + w1 * b[2] + w2 * c[2];
                    if z > depth[y * sw + x] {
                        depth[y * sw + x] = z;
                        img[y * sw + x] = [rgb[0], rgb[1], rgb[2], 255];
                    }
                }
            }
        }
    }
    let mut out = Vec::with_capacity(w * h * 4);
    for y in 0..h {
        for x in 0..w {
            let (mut acc, mut cover) = ([0u32; 3], 0u32);
            for j in 0..SS {
                for i in 0..SS {
                    let p = img[(y * SS + j) * sw + x * SS + i];
                    if p[3] > 0 {
                        cover += 1;
                        for k in 0..3 {
                            acc[k] += p[k] as u32;
                        }
                    }
                }
            }
            if cover == 0 {
                out.extend_from_slice(&[0, 0, 0, 0]);
            } else {
                out.extend_from_slice(&[
                    (acc[0] / cover) as u8,
                    (acc[1] / cover) as u8,
                    (acc[2] / cover) as u8,
                    (cover * 255 / (SS * SS) as u32) as u8,
                ]);
            }
        }
    }
    out
}

fn crc32(data: &[u8]) -> u32 {
    let mut c = 0xffff_ffffu32;
    for &b in data {
        c ^= u32::from(b);
        for _ in 0..8 {
            c = if c & 1 == 1 {
                0xedb8_8320 ^ (c >> 1)
            } else {
                c >> 1
            };
        }
    }
    !c
}

fn deflate(raw: &[u8]) -> Vec<u8> {
    let mut child = Command::new("gzip")
        .args(["-9", "-n", "-c"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .expect("gzip");
    let mut stdin = child.stdin.take().unwrap();
    let data = raw.to_vec();
    let feeder = std::thread::spawn(move || {
        stdin.write_all(&data).unwrap();
    });
    let out = child.wait_with_output().unwrap().stdout;
    feeder.join().unwrap();
    assert!(
        out.len() > 18 && out[0] == 0x1f && out[1] == 0x8b && out[3] == 0,
        "unexpected gzip header"
    );
    let body = &out[10..out.len() - 8]; // the raw deflate stream
    let (mut a, mut b) = (1u32, 0u32);
    for &x in raw {
        a = (a + x as u32) % 65521;
        b = (b + a) % 65521;
    }
    let mut z = vec![0x78, 0xda];
    z.extend_from_slice(body);
    z.extend_from_slice(&((b << 16) | a).to_be_bytes());
    z
}

fn png(rgba: &[u8], w: usize, h: usize) -> Vec<u8> {
    let mut raw = Vec::with_capacity((w * 4 + 1) * h);
    for y in 0..h {
        raw.push(0);
        raw.extend_from_slice(&rgba[y * w * 4..(y + 1) * w * 4]);
    }
    let chunk = |kind: &[u8; 4], data: &[u8], out: &mut Vec<u8>| {
        out.extend_from_slice(&(data.len() as u32).to_be_bytes());
        let mut body = kind.to_vec();
        body.extend_from_slice(data);
        out.extend_from_slice(&body);
        out.extend_from_slice(&crc32(&body).to_be_bytes());
    };
    let mut out = vec![0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a];
    let mut ihdr = Vec::new();
    ihdr.extend_from_slice(&(w as u32).to_be_bytes());
    ihdr.extend_from_slice(&(h as u32).to_be_bytes());
    ihdr.extend_from_slice(&[8, 6, 0, 0, 0]);
    chunk(b"IHDR", &ihdr, &mut out);
    chunk(b"IDAT", &deflate(&raw), &mut out);
    chunk(b"IEND", &[], &mut out);
    out
}

fn base64(bytes: &[u8]) -> String {
    const T: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut s = String::new();
    for c in bytes.chunks(3) {
        let n = (u32::from(c[0]) << 16)
            | (u32::from(*c.get(1).unwrap_or(&0)) << 8)
            | u32::from(*c.get(2).unwrap_or(&0));
        for i in 0..4 {
            s.push(if i <= c.len() {
                T[((n >> (18 - 6 * i)) & 63) as usize] as char
            } else {
                '='
            });
        }
    }
    s
}

fn main() {
    let a: Vec<String> = std::env::args().collect();
    if a.len() < 5 {
        eprintln!("usage: thumbnails IN.gcode OUT.gcode WxH,WxH part.stl:RRGGBB ...");
        std::process::exit(2);
    }
    let parts: Vec<(Vec<[V; 3]>, [f64; 3])> = a[4..]
        .iter()
        .map(|s| {
            let (path, hex) = s.split_once(':').unwrap_or((s, "04D9C4"));
            let v = u32::from_str_radix(hex, 16).expect("colour as RRGGBB");
            (
                stl(path),
                [
                    ((v >> 16) & 255) as f64 / 255.0,
                    ((v >> 8) & 255) as f64 / 255.0,
                    (v & 255) as f64 / 255.0,
                ],
            )
        })
        .collect();
    let gcode = fs::read_to_string(&a[1]).expect("read the G-code");
    let first_end = gcode.find('\n').expect("a G-code file") + 1;
    assert!(
        !gcode.contains("; thumbnail begin"),
        "the file already has thumbnails"
    );
    let mut blocks = String::from("\n");
    for size in a[3].split(',') {
        let (w, h) = size
            .split_once('x')
            .map(|(w, h)| (w.parse::<usize>().unwrap(), h.parse::<usize>().unwrap()))
            .expect("sizes like 313x173");
        let bytes = png(&render(&parts, w, h), w, h);
        let text = base64(&bytes);
        blocks += &format!("; thumbnail begin {w}x{h} {}\n", text.len());
        for line in text.as_bytes().chunks(78) {
            blocks += &format!("; {}\n", std::str::from_utf8(line).unwrap());
        }
        blocks += "; thumbnail end\n";
        println!("{w}x{h}: PNG {} bytes", bytes.len());
    }
    fs::write(
        &a[2],
        format!("{}{}{}", &gcode[..first_end], blocks, &gcode[first_end..]),
    )
    .expect("write the G-code");
}
