use std::env;
use std::fs;
use std::path::PathBuf;

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    let out = PathBuf::from(env::var("OUT_DIR").unwrap());
    let mut s = String::new();

    s.push_str("pub const FINEANGLES: usize = 8192;\n");
    s.push_str("pub const FINE_MASK: usize = FINEANGLES - 1;\n");
    s.push_str("pub const ANG_TO_FINE_SHIFT: u32 = 19;\n");
    s.push_str("pub const SLOPERANGE: u32 = 2048;\n");
    s.push_str("pub const SLOPEBITS: u32 = 11;\n");
    s.push_str("pub const DBITS: u32 = 16 - SLOPEBITS;\n");

    let pi = std::f64::consts::PI;

    // finesine[i] = (int)(FRACUNIT * sin((i + 0.5) * 2PI / FINEANGLES))
    s.push_str("pub static FINESINE: [i32; 10240] = [\n");
    for i in 0..10240 {
        let a = (i as f64 + 0.5) * 2.0 * pi / 8192.0;
        let v = (65536.0 * a.sin()) as i32;
        s.push_str(&format!("{v},"));
    }
    s.push_str("];\n");

    // finetangent[i] = (int)(FRACUNIT * tan((i - FINEANGLES/4 + 0.5)2PI/FINEANGLES))
    s.push_str("pub static FINETANGENT: [i32; 4096] = [\n");
    for i in 0..4096 {
        let a = (i as f64 - 2048.0 + 0.5) * 2.0 * pi / 8192.0;
        let v = (65536.0 * a.tan()) as i32;
        s.push_str(&format!("{v},"));
    }
    s.push_str("];\n");

    // tantoangle[i] = atan(i / SLOPERANGE) scaled to full 2^32 turnout
    s.push_str("pub static TANTOANGLE: [u32; 2049] = [\n");
    for i in 0..2049 {
        let a = (i as f64 / 2048.0).atan();
        let v = (a * (2f64.powi(32) / (2.0 * pi))) as u32;
        s.push_str(&format!("{v},"));
    }
    s.push_str("];\n");

    // gamma correction tables, cosmetic brightness ramps
    s.push_str("pub static GAMMATABLE: [[u8; 256]; 5] = [\n");
    for g in 0..5u32 {
        s.push('[');
        for i in 0..256 {
            let p = (g + 2) as f64 * 0.5;
            let v = (((i + 1) as f64 / 256.0).powf(p) * 256.0).min(255.0) as u8;
            s.push_str(&format!("{v},"));
        }
        s.push_str("],\n");
    }
    s.push_str("];\n");

    fs::write(out.join("tables.rs"), s).unwrap();
}
