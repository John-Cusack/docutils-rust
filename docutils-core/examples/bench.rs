//! Two-track bench helper: `bench <file.rst> <iters>`.
//!
//! Runs `publish_string_with` (report_level 1, matching the Python side of
//! `benches/two_track.py`) `iters` times and prints `elapsed_ms` plus the
//! output length (to prove the work wasn't optimized away).

use docutils_core::core::publish_string_with;
use docutils_core::settings::Settings;

fn main() {
    let path = std::env::args().nth(1).expect("usage: bench <file.rst> <iters>");
    let iters: usize = std::env::args()
        .nth(2)
        .expect("usage: bench <file.rst> <iters>")
        .parse()
        .expect("iters must be a number");
    let source = std::fs::read_to_string(&path).expect("read input");
    let settings = Settings {
        report_level: 1,
        ..Settings::default()
    };
    // Warm-up (allocator, first-parse caches) outside the clock.
    let (first, _) = publish_string_with(&source, &settings, None).expect("bench parse");
    let start = std::time::Instant::now();
    let mut len = 0;
    for _ in 0..iters {
        let (html, _) = publish_string_with(&source, &settings, None).expect("bench parse");
        len = html.len();
    }
    let ms = start.elapsed().as_secs_f64() * 1000.0;
    println!("iters={iters} elapsed_ms={ms:.1} html_len={len} first_len={}", first.len());
}
