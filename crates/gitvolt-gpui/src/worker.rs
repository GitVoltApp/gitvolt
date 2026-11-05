//! What reads a repository's PDFs and draws its Mermaid: this same binary, in a process of its own, as pdf.js has a Web
//! Worker. Those parsers take untrusted bytes and aren't ours to bound from the inside: a crafted file that loops or
//! allocates without end costs only this process, under its own memory limit, and is killed after a time. The app shows
//! the file couldn't be drawn. One worker to a document; it goes with what holds it.

use std::io::{Read, Write};
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};
use std::sync::mpsc;
use std::time::Duration;

const ARG: &str = "--render-worker";
/// The worker's address space, in bytes: room for a big page's canvas and what it holds of the document.
const MEMORY_LIMIT: u64 = 3 << 30;
/// The longest a page or a diagram may take.
pub const REQUEST_TIMEOUT: Duration = Duration::from_secs(20);
const DIAGRAM_TIMEOUT: Duration = Duration::from_secs(10);
/// A diagram's SVG past this isn't a picture anyone reads, and it'd be parsed in the app.
const MAX_SVG_BYTES: usize = 32 << 20;
/// `maxCanvasPixels`: the most pixels a page's canvas gets.
pub const MAX_CANVAS_PIXELS: f64 = 16_777_216.;
/// Mermaid's `maxTextSize`: a diagram's source past this isn't drawn.
pub const MAX_DIAGRAM_BYTES: usize = 50_000;

const OP_OPEN_PDF: u8 = 1;
const OP_RENDER_PAGE: u8 = 2;
const OP_MERMAID: u8 = 3;

fn write_frame(w: &mut impl Write, op: u8, payload: &[u8]) -> std::io::Result<()> {
    w.write_all(&[op])?;
    w.write_all(&(payload.len() as u64).to_le_bytes())?;
    w.write_all(payload)?;
    w.flush()
}

fn read_frame(r: &mut impl Read) -> std::io::Result<(u8, Vec<u8>)> {
    let mut head = [0u8; 9];
    r.read_exact(&mut head)?;
    let len = u64::from_le_bytes(head[1..9].try_into().unwrap_or_default());
    // No frame's larger than a canvas, or the largest file the core loads.
    if len > (1 << 30) {
        return Err(std::io::Error::other("a frame too large"));
    }
    let mut out = vec![0u8; len as usize];
    r.read_exact(&mut out)?;
    Ok((head[0], out))
}

/// One page's image.
pub struct PageImage {
    pub w: u32,
    pub h: u32,
    /// RGBA, on the paper's white.
    pub rgba: Vec<u8>,
}

/// The running worker. Each call blocks until its answer: call it off the UI thread, one at a time.
pub struct Worker {
    child: std::sync::Arc<std::sync::Mutex<Child>>,
    stdin: ChildStdin,
    stdout: ChildStdout,
    dead: bool,
}

impl Worker {
    pub fn spawn() -> std::io::Result<Self> {
        let mut command = Command::new(std::env::current_exe()?);
        command.arg(ARG);
        Self::of(command)
    }

    /// It past a request's end: killed, or fell over.
    pub fn is_dead(&self) -> bool {
        self.dead
    }

    fn of(mut command: Command) -> std::io::Result<Self> {
        let mut child = command.stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::null()).spawn()?;
        let stdin = child.stdin.take().ok_or_else(|| std::io::Error::other("no stdin"))?;
        let stdout = child.stdout.take().ok_or_else(|| std::io::Error::other("no stdout"))?;
        Ok(Self { child: std::sync::Arc::new(std::sync::Mutex::new(child)), stdin, stdout, dead: false })
    }

    /// A request and its answer, or the worker's end: past the timeout it's killed, and its pipe closing ends the read.
    fn call(&mut self, op: u8, payload: &[u8], timeout: Duration) -> Result<Vec<u8>, String> {
        let (done, wait) = mpsc::channel::<()>();
        let child = self.child.clone();
        let watchdog = std::thread::spawn(move || {
            if wait.recv_timeout(timeout) == Err(mpsc::RecvTimeoutError::Timeout)
                && let Ok(mut c) = child.lock()
            {
                let _ = c.kill();
            }
        });
        let answer = write_frame(&mut self.stdin, op, payload).and_then(|()| read_frame(&mut self.stdout));
        let _ = done.send(());
        let _ = watchdog.join();
        match answer {
            Ok((0, bytes)) => Ok(bytes),
            Ok((_, why)) => Err(String::from_utf8_lossy(&why).to_string()),
            Err(_) => {
                self.dead = true;
                Err("it took too long or too much memory".to_string())
            }
        }
    }

    /// Opens a PDF: each page's size in points. `Err`: why it can't be.
    pub fn open_pdf(&mut self, bytes: &[u8]) -> Result<Vec<(f32, f32)>, String> {
        let answer = self.call(OP_OPEN_PDF, bytes, REQUEST_TIMEOUT)?;
        Ok(answer.as_chunks::<8>().0.iter().map(|c| (f32::from_le_bytes([c[0], c[1], c[2], c[3]]), f32::from_le_bytes([c[4], c[5], c[6], c[7]]))).collect())
    }

    pub fn render_page(&mut self, page: u32, scale: f32) -> Result<PageImage, String> {
        let mut req = page.to_le_bytes().to_vec();
        req.extend_from_slice(&scale.to_le_bytes());
        let answer = self.call(OP_RENDER_PAGE, &req, REQUEST_TIMEOUT)?;
        if answer.len() < 8 {
            return Err("no page".to_string());
        }
        let w = u32::from_le_bytes([answer[0], answer[1], answer[2], answer[3]]);
        let h = u32::from_le_bytes([answer[4], answer[5], answer[6], answer[7]]);
        Ok(PageImage { w, h, rgba: answer[8..].to_vec() })
    }

    /// A diagram's SVG.
    pub fn mermaid(&mut self, source: &str, dark: bool) -> Result<String, String> {
        if source.len() > MAX_DIAGRAM_BYTES {
            return Err("it's too long to draw".to_string());
        }
        let mut req = vec![u8::from(dark)];
        req.extend_from_slice(source.as_bytes());
        let answer = self.call(OP_MERMAID, &req, DIAGRAM_TIMEOUT)?;
        if answer.len() > MAX_SVG_BYTES {
            return Err("it's too large to draw".to_string());
        }
        String::from_utf8(answer).map_err(|_| "it isn't an SVG".to_string())
    }
}

impl Drop for Worker {
    fn drop(&mut self) {
        if let Ok(mut c) = self.child.lock() {
            let _ = c.kill();
            let _ = c.wait();
        }
    }
}

// The child's side.

#[cfg(unix)]
fn limit_memory() {
    let limit = libc::rlimit { rlim_cur: MEMORY_LIMIT, rlim_max: MEMORY_LIMIT };
    // SAFETY: setrlimit reads the struct it's given; no core dump, and an address space it can't pass.
    unsafe {
        libc::setrlimit(libc::RLIMIT_AS, &limit);
        let none = libc::rlimit { rlim_cur: 0, rlim_max: 0 };
        libc::setrlimit(libc::RLIMIT_CORE, &none);
    }
}

#[cfg(not(unix))]
fn limit_memory() {}

/// A page's pixels at `scale`, kept under what a canvas gets.
fn draw_page(pdf: &hayro::hayro_syntax::Pdf, page: u32, scale: f32) -> Result<Vec<u8>, String> {
    use hayro::vello_cpu::color::palette::css::WHITE;
    let pages = pdf.pages();
    let p = pages.get(page as usize).ok_or("no such page")?;
    let (w, h) = p.render_dimensions();
    let area = (w as f64 * scale as f64) * (h as f64 * scale as f64);
    let fit = if area > MAX_CANVAS_PIXELS { (MAX_CANVAS_PIXELS / area).sqrt() as f32 } else { 1. };
    let scale = (scale * fit).min(60000. / w.max(h).max(1.));
    let cache = hayro::RenderCache::new();
    let settings = hayro::hayro_interpret::InterpreterSettings::default();
    let render = hayro::RenderSettings::default();
    let pix = hayro::PixmapSettings { x_scale: scale, y_scale: scale, bg_color: WHITE };
    let pixmap = hayro::render(p, &cache, &settings, &render, &pix);
    let (pw, ph) = (pixmap.width() as u32, pixmap.height() as u32);
    let mut out = Vec::with_capacity(8 + (pw * ph * 4) as usize);
    out.extend_from_slice(&pw.to_le_bytes());
    out.extend_from_slice(&ph.to_le_bytes());
    out.extend_from_slice(pixmap.data_as_u8_slice());
    Ok(out)
}

fn serve(op: u8, payload: Vec<u8>, pdf: &mut Option<hayro::hayro_syntax::Pdf>) -> Result<Vec<u8>, String> {
    match op {
        OP_OPEN_PDF => {
            let opened = hayro::hayro_syntax::Pdf::new(payload).map_err(|e| match format!("{e:?}") {
                s if s.contains("assword") || s.contains("ncrypt") => "This PDF is password-protected".to_string(),
                _ => "This PDF couldn't be opened".to_string(),
            })?;
            let mut out = Vec::new();
            for p in opened.pages().iter() {
                let (w, h) = p.render_dimensions();
                out.extend_from_slice(&w.to_le_bytes());
                out.extend_from_slice(&h.to_le_bytes());
            }
            *pdf = Some(opened);
            Ok(out)
        }
        OP_RENDER_PAGE => {
            let doc = pdf.as_ref().ok_or("no document")?;
            if payload.len() < 8 {
                return Err("a bad request".to_string());
            }
            let page = u32::from_le_bytes([payload[0], payload[1], payload[2], payload[3]]);
            let scale = f32::from_le_bytes([payload[4], payload[5], payload[6], payload[7]]);
            draw_page(doc, page, scale)
        }
        OP_MERMAID => {
            let (dark, source) = payload.split_first().ok_or("a bad request")?;
            let source = std::str::from_utf8(source).map_err(|e| e.to_string())?;
            let mut options = mermaid_rs_renderer::RenderOptions::mermaid_default();
            if *dark != 0 {
                options.theme = mermaid_rs_renderer::Theme::dark();
            }
            // On the page, as the web's.
            options.theme.background = "transparent".to_string();
            mermaid_rs_renderer::render_with_options(source, options).map(String::into_bytes).map_err(|e| e.to_string())
        }
        _ => Err("an unknown request".to_string()),
    }
}

/// In `main`, first thing: `true` when this process is a worker, and has done its work.
pub fn run_if_worker() -> bool {
    if std::env::args().nth(1).as_deref() != Some(ARG) {
        return false;
    }
    limit_memory();
    let (mut input, mut output) = (std::io::stdin().lock(), std::io::stdout().lock());
    let mut pdf = None;
    while let Ok((op, payload)) = read_frame(&mut input) {
        let (status, body) = match serve(op, payload, &mut pdf) {
            Ok(b) => (0, b),
            Err(why) => (1, why.into_bytes()),
        };
        if write_frame(&mut output, status, &body).is_err() {
            break;
        }
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::prelude::v1::test;

    /// A PDF of one blue square, by hand.
    pub fn small_pdf() -> Vec<u8> {
        let objs = ["<< /Type /Catalog /Pages 2 0 R >>", "<< /Type /Pages /Kids [3 0 R] /Count 1 >>", "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 200 100] /Contents 4 0 R >>", "<< /Length 28 >>\nstream\n0 0 1 rg 10 10 50 50 re f   \nendstream"];
        let mut out = b"%PDF-1.4\n".to_vec();
        let mut offs = Vec::new();
        for (i, o) in objs.iter().enumerate() {
            offs.push(out.len());
            out.extend_from_slice(format!("{} 0 obj\n{o}\nendobj\n", i + 1).as_bytes());
        }
        let x = out.len();
        out.extend_from_slice(format!("xref\n0 {}\n0000000000 65535 f \n", objs.len() + 1).as_bytes());
        for o in offs {
            out.extend_from_slice(format!("{o:010} 00000 n \n").as_bytes());
        }
        out.extend_from_slice(format!("trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{x}\n%%EOF\n", objs.len() + 1).as_bytes());
        out
    }

    #[test]
    fn a_page_draws_on_white() {
        let mut pdf = None;
        let sizes = serve(OP_OPEN_PDF, small_pdf(), &mut pdf).unwrap();
        assert_eq!(sizes.len(), 8, "one page");
        let mut req = 0u32.to_le_bytes().to_vec();
        req.extend_from_slice(&1f32.to_le_bytes());
        let img = serve(OP_RENDER_PAGE, req, &mut pdf).unwrap();
        let (w, h) = (u32::from_le_bytes(img[0..4].try_into().unwrap()), u32::from_le_bytes(img[4..8].try_into().unwrap()));
        assert_eq!((w, h), (200, 100));
        let at = |x: usize, y: usize| &img[8 + (y * 200 + x) * 4..8 + (y * 200 + x) * 4 + 3];
        assert_eq!(at(150, 50), [255, 255, 255], "paper");
        assert_eq!(at(30, 60), [0, 0, 255], "the square");
    }

    #[test]
    fn what_is_no_pdf_says_so() {
        let mut pdf = None;
        assert!(serve(OP_OPEN_PDF, b"nothing like one".to_vec(), &mut pdf).is_err());
        assert!(serve(OP_RENDER_PAGE, vec![0; 8], &mut pdf).is_err(), "no document yet");
    }

    #[test]
    fn a_huge_page_stays_within_a_canvas() {
        let pdf_src = String::from_utf8(small_pdf()).unwrap().replace("[0 0 200 100]", "[0 0 14400 14400]");
        let mut pdf = None;
        // The offsets moved: it's read as a damaged file, as pdf.js would. Either it opens or it says so.
        if serve(OP_OPEN_PDF, pdf_src.into_bytes(), &mut pdf).is_ok() {
            let mut req = 0u32.to_le_bytes().to_vec();
            req.extend_from_slice(&10f32.to_le_bytes());
            let img = serve(OP_RENDER_PAGE, req, &mut pdf).unwrap();
            let (w, h) = (u32::from_le_bytes(img[0..4].try_into().unwrap()) as u64, u32::from_le_bytes(img[4..8].try_into().unwrap()) as u64);
            assert!(w * h <= MAX_CANVAS_PIXELS as u64 + 200_000, "{w}x{h}");
        }
    }

    #[test]
    #[cfg(unix)]
    fn a_worker_that_hangs_is_killed_and_says_so() {
        let mut command = Command::new("sh");
        command.args(["-c", "exec sleep 60"]);
        let mut w = Worker::of(command).unwrap();
        let start = std::time::Instant::now();
        assert!(w.call(OP_MERMAID, b"x", Duration::from_millis(200)).is_err());
        assert!(start.elapsed() < Duration::from_secs(10), "{:?}", start.elapsed());
        assert!(w.is_dead());
    }

    #[test]
    #[cfg(unix)]
    fn a_worker_that_falls_over_says_so() {
        let mut command = Command::new("sh");
        command.args(["-c", "exit 3"]);
        let mut w = Worker::of(command).unwrap();
        assert!(w.call(OP_MERMAID, b"x", Duration::from_secs(5)).is_err());
        assert!(w.is_dead());
    }

    #[test]
    fn a_frame_round_trips_and_a_lying_length_is_refused() {
        let mut buf = Vec::new();
        write_frame(&mut buf, 7, b"abc").unwrap();
        let (op, body) = read_frame(&mut &buf[..]).unwrap();
        assert_eq!((op, body.as_slice()), (7, &b"abc"[..]));
        let mut lie = vec![1u8];
        lie.extend_from_slice(&u64::MAX.to_le_bytes());
        assert!(read_frame(&mut &lie[..]).is_err());
    }

    #[test]
    fn a_diagram_draws() {
        let mut pdf = None;
        let mut req = vec![1u8];
        req.extend_from_slice(b"graph TD\n  A[Quay] --> B[Gate]\n");
        let svg = String::from_utf8(serve(OP_MERMAID, req, &mut pdf).unwrap()).unwrap();
        assert!(svg.contains("<svg") && svg.contains("Quay"));
    }
}
