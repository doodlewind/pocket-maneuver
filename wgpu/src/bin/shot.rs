//! Frames of the game written to files, on the build machine: the renderer
//! of the browser tab on this machine's GPU, reading the pack from disk.
//!
//!   maneuver-shot --pack world.pack --out frame.png [--shape vita] [--size 960x544] [--frames 240]
//!                 [--words "mode=play auto=1 …"] [--cue 300:"view=…"]… [--status status.json] [--log 60]
//!   maneuver-shot --pack world.pack --film frames.rgba --frames 900 [--from 600] …
//!   maneuver-shot --compare a.png b.png
//!
//! `--pack` names the pack's file, or the manifest (`.json`) of a pack cut
//! into pieces. The game runs `--frames` frames of a sixtieth of a second
//! each, with no interface: behind the title the autopilot flies the route,
//! and `--words` are a development host's (`App::control`): `mode=play
//! auto=1` is play flown by the autopilot, with the marks on the world.
//! `--cue N:WORDS` sends words before frame N. `--out` is the last frame as a
//! PNG. `--film` is every frame from `--from` on as rows of RGBA, one frame
//! after another (`-` for the standard output), for an encoder to read.
//! `--log N` prints the status every N frames to the standard error.
//!
//! `--compare` says how far two pictures of one size are apart: the mean
//! difference of a colour in 255ths and the share of pixels where a colour
//! differs by more than 16.

#[cfg(not(target_arch = "wasm32"))]
fn main() {
    if let Err(e) = native::run() {
        eprintln!("maneuver-shot: {e}");
        std::process::exit(1);
    }
}

#[cfg(target_arch = "wasm32")]
fn main() {}

#[cfg(not(target_arch = "wasm32"))]
mod native {
    use std::io::Write;

    use maneuver_wgpu::app::{App, Held, Shape, World};
    use maneuver_wgpu::pack::Progress;
    use pocket_web_wgpu::gpu::{Gpu, Screen};
    use pocket_web_wgpu::source::Source;
    use pocket_web_wgpu::task;

    fn options(name: &str) -> Vec<String> {
        let args: Vec<String> = std::env::args().collect();
        args.iter().enumerate().filter(|(_, a)| *a == name).filter_map(|(i, _)| args.get(i + 1).cloned()).collect()
    }

    fn option(name: &str) -> Option<String> {
        options(name).into_iter().next()
    }

    /// A PNG file as rows of RGBA.
    fn picture(path: &str) -> Result<Vec<u8>, String> {
        let mut decoder = png::Decoder::new(std::fs::File::open(path).map_err(|e| format!("{path}: {e}"))?);
        decoder.set_transformations(png::Transformations::EXPAND | png::Transformations::STRIP_16);
        let mut reader = decoder.read_info().map_err(|e| format!("{path}: {e}"))?;
        let mut bytes = vec![0; reader.output_buffer_size()];
        let info = reader.next_frame(&mut bytes).map_err(|e| format!("{path}: {e}"))?;
        bytes.truncate(info.buffer_size());
        Ok(match info.color_type {
            png::ColorType::Rgba => bytes,
            png::ColorType::Rgb => bytes.chunks_exact(3).flat_map(|p| [p[0], p[1], p[2], 255]).collect(),
            png::ColorType::Grayscale => bytes.iter().flat_map(|&g| [g, g, g, 255]).collect(),
            png::ColorType::GrayscaleAlpha => bytes.chunks_exact(2).flat_map(|p| [p[0], p[0], p[0], p[1]]).collect(),
            png::ColorType::Indexed => return Err(format!("{path}: a palette the decoder did not expand")),
        })
    }

    /// How far two pictures of one size are apart.
    fn apart(a: &[u8], b: &[u8]) -> (f64, f64) {
        let (mut sum, mut far) = (0u64, 0u32);
        for (a, b) in a.chunks_exact(4).zip(b.chunks_exact(4)) {
            let d = [0, 1, 2].map(|c| a[c].abs_diff(b[c]));
            sum += d.iter().map(|&d| d as u64).sum::<u64>();
            far += d.iter().any(|&d| d > 16) as u32;
        }
        let count = (a.len() / 4) as f64;
        (sum as f64 / (count * 3.0), far as f64 / count)
    }

    pub fn run() -> Result<(), String> {
        if let Some(first) = option("--compare") {
            let second = std::env::args().skip_while(|a| a != "--compare").nth(2).ok_or("--compare A.png B.png")?;
            let (a, b) = (picture(&first)?, picture(&second)?);
            if a.len() != b.len() {
                return Err("the two pictures are not of one size".into());
            }
            let (mean, over) = apart(&a, &b);
            println!("{{\"mean\":{mean:.3},\"over16\":{over:.4}}}");
            return Ok(());
        }
        let pack = option("--pack").ok_or("--pack PATH")?;
        let mut shape = Shape::named(&option("--shape").unwrap_or("vita".into())).ok_or("--shape vita | psp | 3ds | ipod")?;
        if let Some(size) = option("--size") {
            let (w, h) = size.split_once('x').ok_or("--size WIDTHxHEIGHT")?;
            (shape.width, shape.height) = (w.parse().map_err(|_| "--size WIDTHxHEIGHT")?, h.parse().map_err(|_| "--size WIDTHxHEIGHT")?);
        }
        let number = |name: &str, fallback: u32| option(name).map_or(Ok(fallback), |v| v.parse::<u32>().map_err(|_| format!("{name} takes a number")));
        let (frames, from, log) = (number("--frames", 240)?, number("--from", 0)?, number("--log", 0)?);
        let mut cues: Vec<(u32, String)> = Vec::new();
        for cue in options("--cue") {
            let (at, words) = cue.split_once(':').ok_or("--cue FRAME:WORDS")?;
            cues.push((at.parse().map_err(|_| "--cue FRAME:WORDS")?, words.into()));
        }
        let (out, film) = (option("--out"), option("--film"));
        if out.is_none() && film.is_none() {
            return Err("--out PNG or --film FILE".into());
        }

        let gpu = task::wait(Gpu::headless())?;
        let screen = Screen::texture(&gpu, shape.width, shape.height, 1);
        let source = task::wait(Source::open(&pack))?;
        let world = task::wait(World::read(gpu.clone(), screen.format, source, Progress::default()))?;
        let mut app = App::open(gpu, screen, shape);
        app.fly(world);
        if let Some(words) = option("--words") {
            app.control(&words);
        }
        let mut sink: Option<Box<dyn Write>> = match film.as_deref() {
            Some("-") => Some(Box::new(std::io::BufWriter::new(std::io::stdout().lock()))),
            Some(path) => Some(Box::new(std::io::BufWriter::new(std::fs::File::create(path).map_err(|e| format!("{path}: {e}"))?))),
            None => None,
        };
        let pad = Held::default();
        let mut pixels = Vec::new();
        for count in 0..frames {
            for (_, words) in cues.iter().filter(|(at, _)| *at == count) {
                app.control(words);
            }
            app.frame(count as f64 * 1000.0 / 60.0, &pad)?;
            if log > 0 && count % log == 0 {
                eprintln!("{count} {}", app.status());
            }
            if let (Some(sink), true) = (&mut sink, count >= from) {
                pixels = task::wait(app.screen.read(&app.gpu))?;
                sink.write_all(&pixels).map_err(|e| format!("the film: {e}"))?;
            }
        }
        if let Some(mut sink) = sink {
            sink.flush().map_err(|e| format!("the film: {e}"))?;
        }
        if let Some(out) = &out {
            if film.is_none() || pixels.is_empty() {
                pixels = task::wait(app.screen.read(&app.gpu))?;
            }
            let file = std::fs::File::create(out).map_err(|e| format!("{out}: {e}"))?;
            let mut encoder = png::Encoder::new(std::io::BufWriter::new(file), shape.width, shape.height);
            encoder.set_color(png::ColorType::Rgba);
            encoder.set_depth(png::BitDepth::Eight);
            encoder.write_header().and_then(|mut w| w.write_image_data(&pixels)).map_err(|e| format!("{out}: {e}"))?;
        }
        let status = app.status();
        if let Some(path) = option("--status") {
            std::fs::write(&path, &status).map_err(|e| format!("{path}: {e}"))?;
        }
        if film.as_deref() != Some("-") {
            println!("{status}");
        }
        Ok(())
    }
}
