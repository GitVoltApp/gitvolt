//! Audio, video and fonts (`ui/src/media/`), laid out as the image diff is. GPUI has no player: a sound or a film shows
//! what its bytes say, with Open in and Show hex. A font is drawn from its own outlines, never added to the app's fonts:
//! a repository's font can't change what the app's text looks like.

use crate::app::GitVolt;
use crate::theme;
use crate::ui::tooltip::tip;
use gpui::prelude::*;
use gpui::*;
use std::cell::RefCell;
use std::io::Read;
use std::rc::Rc;
use std::sync::Arc;

pub const TOOLBAR_H: f32 = 33.;
const PANGRAM: &str = "The quick brown fox jumps over the lazy dog";
const SAMPLE_SIZES: [f32; 5] = [12., 18., 24., 36., 48.];
/// A packed table that unpacks past this is hostile.
const MAX_FONT_TABLE: u64 = 64 << 20;
const MAX_NAME_TABLE: u64 = 1 << 20;
const MAX_NAME_BYTES: usize = 1024;
/// A sample's outlines, as SVG path data: past this a crafted font's glyphs aren't drawn.
const MAX_OUTLINE_BYTES: usize = 8 << 20;
/// A sample's picture, each way: a crafted font's advances can't ask for more.
const MAX_SAMPLE_PX: f32 = 8192.;
/// `body`'s line height, each sample's.
const LINE_HEIGHT: f32 = 1.4;
/// The size column and its gap, `.font-size` and `.font-line`'s.
const LABEL_W: f32 = 22.;
const GAP: f32 = 10.;
/// A scrollbar's gutter.
const GUTTER: f32 = 10.;
const PAD: f32 = 16.;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Audio,
    Video,
    Font,
}

/// By extension; the core's `MEDIA_EXTENSIONS` is the same list.
pub fn media_of(path: &str) -> Option<(Kind, &'static str)> {
    let name = path.rsplit('/').next().unwrap_or(path);
    let ext = name.rsplit_once('.')?.1.to_ascii_lowercase();
    Some(match ext.as_str() {
        "mp3" => (Kind::Audio, "audio/mpeg"),
        "wav" => (Kind::Audio, "audio/wav"),
        "flac" => (Kind::Audio, "audio/flac"),
        "ogg" | "oga" => (Kind::Audio, "audio/ogg"),
        "opus" => (Kind::Audio, "audio/ogg; codecs=opus"),
        "m4a" => (Kind::Audio, "audio/mp4"),
        "aac" => (Kind::Audio, "audio/aac"),
        "weba" => (Kind::Audio, "audio/webm"),
        "webm" => (Kind::Video, "video/webm"),
        "ogv" => (Kind::Video, "video/ogg"),
        "mp4" | "m4v" => (Kind::Video, "video/mp4"),
        "mov" => (Kind::Video, "video/quicktime"),
        "ttf" => (Kind::Font, "font/ttf"),
        "otf" => (Kind::Font, "font/otf"),
        "woff" => (Kind::Font, "font/woff"),
        "woff2" => (Kind::Font, "font/woff2"),
        _ => return None,
    })
}

const FOURCC: [(&[u8; 4], &str); 6] = [(b"avc1", "H.264"), (b"avc3", "H.264"), (b"hvc1", "HEVC"), (b"hev1", "HEVC"), (b"av01", "AV1"), (b"vp09", "VP9")];
const SCAN: usize = 4 << 20;

/// The video codec an MP4 or QuickTime file names, or `None`.
pub fn video_codec(bytes: &[u8]) -> Option<&'static str> {
    if bytes.len() < 12 || &bytes[4..8] != b"ftyp" {
        return None;
    }
    let find = |part: &[u8]| part.windows(4).find_map(|w| FOURCC.iter().find(|(c, _)| *c == w).map(|(_, n)| *n));
    find(&bytes[..bytes.len().min(SCAN)]).or_else(|| (bytes.len() > SCAN).then(|| find(&bytes[bytes.len().saturating_sub(SCAN).max(SCAN)..])).flatten())
}

/// What the note names: the codec, else the container.
pub fn video_format(mime: &str, codec: Option<&str>) -> String {
    if let Some(c) = codec {
        return c.to_string();
    }
    match mime {
        "video/mp4" => "MP4",
        "video/quicktime" => "QuickTime",
        "video/webm" => "WebM",
        "video/ogg" | "audio/ogg" => "Ogg",
        "audio/mpeg" => "MP3",
        "audio/wav" => "WAV",
        "audio/flac" => "FLAC",
        "audio/ogg; codecs=opus" => "Opus",
        "audio/mp4" => "MP4 audio",
        "audio/aac" => "AAC",
        "audio/webm" => "WebM",
        other => other,
    }
    .to_string()
}

fn u16_le(b: &[u8], at: usize) -> Option<u32> {
    Some(u16::from_le_bytes([*b.get(at)?, *b.get(at + 1)?]) as u32)
}

fn u32_le(b: &[u8], at: usize) -> Option<u32> {
    Some(u32::from_le_bytes(b.get(at..at + 4)?.try_into().ok()?))
}

fn u32_be(b: &[u8], at: usize) -> Option<u32> {
    Some(u32::from_be_bytes(b.get(at..at + 4)?.try_into().ok()?))
}

fn seconds(s: f64) -> String {
    if s < 60. { format!("{s:.1} s") } else { format!("{}:{:02}", (s / 60.) as u64, (s % 60.) as u64) }
}

fn channels(n: u32) -> String {
    match n {
        1 => "mono".into(),
        2 => "stereo".into(),
        n => format!("{n} channels"),
    }
}

/// A WAV's rate, channels and length, from its chunks.
fn wav_info(b: &[u8]) -> Option<String> {
    if b.get(..4)? != b"RIFF" || b.get(8..12)? != b"WAVE" {
        return None;
    }
    let (mut at, mut fmt, mut data) = (12usize, None, None);
    // Chunks to the end, at most a few: a file of a million empty chunks isn't walked.
    for _ in 0..64 {
        let (id, len) = (b.get(at..at + 4)?, u32_le(b, at + 4)? as usize);
        match id {
            b"fmt " => fmt = Some((u16_le(b, at + 10)?, u32_le(b, at + 12)?, u32_le(b, at + 16)?)),
            b"data" => data = Some(len),
            _ => {}
        }
        if fmt.is_some() && data.is_some() {
            break;
        }
        at = at.checked_add(8)?.checked_add(len + (len & 1))?;
    }
    let (ch, rate, per_sec) = fmt?;
    let mut parts = vec![format!("{rate} Hz"), channels(ch)];
    if let Some(d) = data.filter(|_| per_sec > 0) {
        parts.insert(0, seconds(d as f64 / per_sec as f64));
    }
    Some(parts.join(" · "))
}

/// A FLAC's STREAMINFO.
fn flac_info(b: &[u8]) -> Option<String> {
    if b.get(..4)? != b"fLaC" {
        return None;
    }
    let s = b.get(8..26)?;
    let rate = ((s[10] as u32) << 12) | ((s[11] as u32) << 4) | ((s[12] as u32) >> 4);
    let ch = ((s[12] as u32 >> 1) & 7) + 1;
    let samples = ((s[13] as u64 & 0xf) << 32) | ((s[14] as u64) << 24) | ((s[15] as u64) << 16) | ((s[16] as u64) << 8) | s[17] as u64;
    let mut parts = vec![format!("{rate} Hz"), channels(ch)];
    if rate > 0 && samples > 0 {
        parts.insert(0, seconds(samples as f64 / rate as f64));
    }
    Some(parts.join(" · "))
}

/// An MP4's or QuickTime's length and picture size, from its `mvhd` and `tkhd`.
fn mp4_info(b: &[u8]) -> Option<String> {
    if b.get(4..8)? != b"ftyp" {
        return None;
    }
    let window = &b[..b.len().min(SCAN)];
    let find = |tag: &[u8; 4]| window.windows(4).position(|w| w == tag);
    let mut parts = Vec::new();
    if let Some(at) = find(b"mvhd") {
        let version = *b.get(at + 4)?;
        let (scale, duration) = if version == 1 { (u32_be(b, at + 24)?, ((u32_be(b, at + 28)? as u64) << 32) | u32_be(b, at + 32)? as u64) } else { (u32_be(b, at + 16)?, u32_be(b, at + 20)? as u64) };
        if scale > 0 {
            parts.push(seconds(duration as f64 / scale as f64));
        }
    }
    if let Some(at) = find(b"tkhd") {
        let version = *b.get(at + 4)?;
        let base = if version == 1 { at + 4 + 88 } else { at + 4 + 76 };
        let (w, h) = (u32_be(b, base)? >> 16, u32_be(b, base + 4)? >> 16);
        if w > 0 && h > 0 {
            parts.push(format!("{w}×{h}"));
        }
    }
    (!parts.is_empty()).then(|| parts.join(" · "))
}

/// What a file's bytes say of it, past its format.
pub fn describe(bytes: &[u8]) -> Option<String> {
    wav_info(bytes).or_else(|| flac_info(bytes)).or_else(|| mp4_info(bytes))
}

/// `name` table name ids.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FontName {
    pub family: String,
    pub style: String,
}

fn be16(b: &[u8], at: usize) -> Option<usize> {
    Some(u16::from_be_bytes([*b.get(at)?, *b.get(at + 1)?]) as usize)
}

fn inflate(data: &[u8], limit: u64) -> Option<Vec<u8>> {
    let mut out = Vec::new();
    flate2::read::ZlibDecoder::new(data).take(limit + 1).read_to_end(&mut out).ok()?;
    (out.len() as u64 <= limit).then_some(out)
}

/// Unicode and Windows names are UTF-16BE; Macintosh Roman, read as Latin-1.
fn decode_name(platform: usize, raw: &[u8]) -> String {
    if platform == 1 {
        return raw.iter().map(|b| *b as char).collect();
    }
    let units: Vec<u16> = raw.as_chunks::<2>().0.iter().map(|c| u16::from_be_bytes(*c)).collect();
    String::from_utf16_lossy(&units)
}

/// Ids 16 and 17 over 1 and 2; Windows English over the rest.
pub fn read_names(t: &[u8]) -> Option<FontName> {
    let (count, strings) = (be16(t, 2)?, be16(t, 4)?);
    let mut best: [Option<(u8, String)>; 4] = Default::default();
    let mut seen = std::collections::HashSet::new();
    for i in 0..count {
        let at = 6 + i * 12;
        if at + 12 > t.len() {
            break;
        }
        let (platform, language, id, length, offset) = (be16(t, at)?, be16(t, at + 4)?, be16(t, at + 6)?, be16(t, at + 8)?, be16(t, at + 10)?);
        let slot = match id {
            1 => 0,
            2 => 1,
            16 => 2,
            17 => 3,
            _ => continue,
        };
        if !matches!(platform, 0 | 1 | 3) || length > MAX_NAME_BYTES {
            continue;
        }
        let Some(raw) = t.get(strings + offset..strings + offset + length) else { continue };
        let rank = if platform == 3 { u8::from(language != 0x409) } else if platform == 0 { 2 } else { 3 };
        // Each once, even when it came out empty: 65,535 records of a blank string cost one decode.
        if !seen.insert((slot, rank)) {
            continue;
        }
        if best[slot].as_ref().is_none_or(|b| rank < b.0) {
            let text = decode_name(platform, raw).trim().to_string();
            if !text.is_empty() {
                best[slot] = Some((rank, text));
            }
        }
    }
    let pick = |a: usize, b: usize| best[a].as_ref().or(best[b].as_ref()).map(|x| x.1.clone());
    Some(FontName { family: pick(2, 0)?, style: pick(3, 1).unwrap_or_default() })
}

/// Its sfnt: a TrueType or OpenType file as it is, a WOFF rebuilt from its packed tables. WOFF2 packs all its tables in
/// one Brotli stream with transforms of their own: `None`.
pub fn sfnt_of(bytes: &[u8]) -> Option<std::borrow::Cow<'_, [u8]>> {
    match bytes.get(..4)? {
        b"wOFF" => {}
        b"OTTO" | b"true" | [0, 1, 0, 0] => return Some(bytes.into()),
        _ => return None,
    }
    let n = be16(bytes, 12)?;
    let mut tables: Vec<([u8; 4], u32, Vec<u8>)> = Vec::with_capacity(n.min(256));
    let mut total = 0u64;
    for i in 0..n {
        let at = 44 + i * 20;
        let tag: [u8; 4] = bytes.get(at..at + 4)?.try_into().ok()?;
        let (offset, comp, orig, sum) = (u32_be(bytes, at + 4)? as usize, u32_be(bytes, at + 8)? as usize, u32_be(bytes, at + 12)? as u64, u32_be(bytes, at + 16)?);
        let raw = bytes.get(offset..offset.checked_add(comp)?)?;
        total += orig;
        if total > MAX_FONT_TABLE {
            return None;
        }
        let data = if (comp as u64) < orig { inflate(raw, orig)? } else { raw.to_vec() };
        tables.push((tag, sum, data));
    }
    let flavour = bytes.get(4..8)?;
    let mut out = flavour.to_vec();
    let count = tables.len() as u16;
    let (mut power, mut log) = (1u16, 0u16);
    while power.saturating_mul(2) <= count {
        power *= 2;
        log += 1;
    }
    for v in [count, power * 16, log, count * 16 - power * 16] {
        out.extend_from_slice(&v.to_be_bytes());
    }
    let mut offset = 12 + tables.len() as u32 * 16;
    tables.sort_by_key(|t| t.0);
    for (tag, sum, data) in &tables {
        out.extend_from_slice(tag);
        out.extend_from_slice(&sum.to_be_bytes());
        out.extend_from_slice(&offset.to_be_bytes());
        out.extend_from_slice(&(data.len() as u32).to_be_bytes());
        offset += (data.len() as u32 + 3) & !3;
    }
    for (_, _, data) in &tables {
        out.extend_from_slice(data);
        out.resize((out.len() + 3) & !3, 0);
    }
    Some(out.into())
}

/// The `name` table's family and style.
pub fn font_name(sfnt: &[u8]) -> Option<FontName> {
    let n = be16(sfnt, 4)?;
    for i in 0..n {
        let at = 12 + i * 16;
        if sfnt.get(at..at + 4)? == b"name" {
            let (offset, length) = (u32_be(sfnt, at + 8)? as usize, u32_be(sfnt, at + 12)? as u64);
            if length > MAX_NAME_TABLE {
                return None;
            }
            return read_names(sfnt.get(offset..offset + length as usize)?);
        }
    }
    None
}

struct Outline<'a> {
    d: &'a mut String,
    x: f32,
    y: f32,
    k: f32,
}

impl ttf_parser::OutlineBuilder for Outline<'_> {
    fn move_to(&mut self, x: f32, y: f32) {
        self.d.push_str(&format!("M{:.2} {:.2}", self.x + x * self.k, self.y - y * self.k));
    }
    fn line_to(&mut self, x: f32, y: f32) {
        self.d.push_str(&format!("L{:.2} {:.2}", self.x + x * self.k, self.y - y * self.k));
    }
    fn quad_to(&mut self, a: f32, b: f32, x: f32, y: f32) {
        self.d.push_str(&format!("Q{:.2} {:.2} {:.2} {:.2}", self.x + a * self.k, self.y - b * self.k, self.x + x * self.k, self.y - y * self.k));
    }
    fn curve_to(&mut self, a: f32, b: f32, c: f32, d: f32, x: f32, y: f32) {
        self.d.push_str(&format!("C{:.2} {:.2} {:.2} {:.2} {:.2} {:.2}", self.x + a * self.k, self.y - b * self.k, self.x + c * self.k, self.y - d * self.k, self.x + x * self.k, self.y - y * self.k));
    }
    fn close(&mut self) {
        self.d.push('Z');
    }
}

/// `text`'s outlines at `size`, from x and baseline y, into SVG path data; the width it took.
fn text_path(face: &ttf_parser::Face, text: &str, size: f32, x: f32, y: f32, d: &mut String) -> f32 {
    let k = size / face.units_per_em() as f32;
    let mut at = x;
    for c in text.chars() {
        let Some(id) = face.glyph_index(c) else {
            at += size * 0.5;
            continue;
        };
        if d.len() < MAX_OUTLINE_BYTES {
            face.outline_glyph(id, &mut Outline { d, x: at, y, k });
        }
        at += face.glyph_hor_advance(id).unwrap_or(0) as f32 * k;
    }
    at - x
}

/// Each line's size and baseline; the grid's top; the grid's top, cell width and height, and columns; the height.
type SampleLayout = (Vec<(f32, f32)>, f32, (f32, f32, f32, usize), f32);

/// Each sample's line box and the glyphs' grid, for `width` of content.
fn sample_layout(asc: f32, desc: f32, width: f32) -> SampleLayout {
    let mut y = 0.;
    let mut lines = Vec::new();
    for size in SAMPLE_SIZES {
        let lh = size * LINE_HEIGHT;
        lines.push((size, y + (lh - (asc + desc) * size) / 2. + asc * size));
        y += lh + GAP;
    }
    // `repeat(auto-fill, minmax(2.2em, 1fr))` at 22 px, a 2 px gap; a box 1.8em tall and its border. The gap, and 8 more.
    let grid_top = y + 8.;
    let (cell_min, gap, cell_h) = (22. * 2.2, 2., 22. * 1.8 + 2.);
    let per = ((width + gap) / (cell_min + gap)).floor().max(1.) as usize;
    let cell_w = (width - (per as f32 - 1.) * gap) / per as f32;
    let rows = 94usize.div_ceil(per);
    let h = grid_top + rows as f32 * (cell_h + gap) - gap;
    (lines, grid_top, (grid_top, cell_w, cell_h, per), h)
}

/// A pangram at each size, and Basic Latin's glyphs, as an image in `color`. The sizes are the UI's text, drawn apart.
pub struct FontSample {
    pub image: Arc<RenderImage>,
    /// The picture's size: the widest sample line can pass the pane's width, and the pane scrolls to it.
    pub w: f32,
    pub h: f32,
    /// Each sample line's size and baseline, for its label; and the glyph grid's top, cell width, height and columns.
    pub lines: Vec<(f32, f32)>,
    pub grid: (f32, f32, f32, usize),
    /// The pane's room for it, with a gutter taken when it's taller than the pane.
    pub content_w: f32,
}

/// Draws the samples for a pane `width` wide, under `above` of what's over them, in a view `view_h` tall. `None`: no
/// outlines to draw.
pub fn font_sample(sfnt: &[u8], width: f32, above: f32, view_h: f32, color: &Rgba, svg: &SvgRenderer) -> Option<FontSample> {
    let face = ttf_parser::Face::parse(sfnt, 0).ok()?;
    let em = (face.units_per_em() as f32).max(1.);
    let (asc, desc) = (face.ascender() as f32 / em, -(face.descender() as f32) / em);
    let mut content_w = width;
    let mut laid = sample_layout(asc, desc, content_w);
    // The gutter's there only when the pane scrolls, and taking it can add a row.
    if above + laid.3 + PAD > view_h {
        content_w = width - GUTTER;
        laid = sample_layout(asc, desc, content_w);
    }
    let (lines, grid_top, grid, content_h) = laid;
    let (_, cell_w, cell_h, per) = grid;
    let mut d = String::new();
    let mut widest = content_w;
    for (size, base) in &lines {
        widest = widest.max(LABEL_W + text_path(&face, PANGRAM, *size, LABEL_W, *base, &mut d));
    }
    let k = 22. / em;
    // A glyph's line box, centred in its box.
    let base = 1. + (cell_h - 2.) / 2. - (asc + desc) * 11. + asc * 22.;
    for (i, c) in ('\u{21}'..='\u{7e}').enumerate() {
        let (x, y) = ((i % per) as f32 * (cell_w + 2.), (i / per) as f32 * (cell_h + 2.) + grid_top);
        let Some(id) = face.glyph_index(c) else { continue };
        let adv = face.glyph_hor_advance(id).unwrap_or(0) as f32 * k;
        if d.len() < MAX_OUTLINE_BYTES {
            face.outline_glyph(id, &mut Outline { d: &mut d, x: x + (cell_w - adv) / 2., y: y + base, k });
        }
    }
    let (w, h) = (widest.min(MAX_SAMPLE_PX).ceil(), content_h.min(MAX_SAMPLE_PX).ceil());
    let fill = format!("#{:02x}{:02x}{:02x}", (color.r * 255.).round() as u8, (color.g * 255.).round() as u8, (color.b * 255.).round() as u8);
    let doc = format!("<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"{w}\" height=\"{h}\" viewBox=\"0 0 {w} {h}\"><path fill=\"{fill}\" fill-opacity=\"{}\" d=\"{d}\"/></svg>", color.a);
    let image = svg.render_single_frame(doc.as_bytes(), 1.).ok()?;
    Some(FontSample { image, w, h, lines, grid, content_w })
}

pub struct MediaSide {
    pub kind: Kind,
    pub mime: &'static str,
    pub size: u64,
    pub codec: Option<&'static str>,
    /// What its bytes say of it.
    pub info: Option<String>,
    /// A font's.
    pub sfnt: Option<Arc<Vec<u8>>>,
    pub name: Option<FontName>,
    pub sample: Option<Arc<FontSample>>,
    pub sample_for: (f32, f32),
    pub drawing: bool,
    pub failed: bool,
    pub theme: u64,
}

impl MediaSide {
    /// Reads what's cheap from a side's bytes, off the UI thread.
    pub fn of(bytes: &[u8], size: u64, kind: Kind, mime: &'static str) -> Self {
        let mut side = Self { kind, mime, size, codec: None, info: None, sfnt: None, name: None, sample: None, sample_for: (0., 0.), drawing: false, failed: false, theme: 0 };
        match kind {
            Kind::Font => match sfnt_of(bytes) {
                Some(s) if ttf_parser::Face::parse(&s, 0).is_ok() => {
                    side.name = font_name(&s);
                    side.sfnt = Some(Arc::new(s.into_owned()));
                }
                _ => side.failed = true,
            },
            Kind::Video | Kind::Audio => {
                side.codec = video_codec(bytes);
                side.info = describe(bytes);
            }
        }
        side
    }
}

pub struct MediaView {
    pub old: Option<MediaSide>,
    pub new: Option<MediaSide>,
    /// File View: the one version, the new side's if there is one.
    pub file: bool,
    pub source_on: bool,
    pub retired: Rc<RefCell<Vec<Arc<RenderImage>>>>,
    /// The stage's size, and each pane's scroll: down and across. Font panes scroll apart, as the web's.
    pub stage: (f32, f32),
    pub scroll: [(f32, f32); 2],
    drag: Option<(usize, bool, f32, f32)>,
    /// Where the stage is on screen, for a press.
    stage_origin: Rc<std::cell::Cell<(f32, f32)>>,
}

impl Drop for MediaView {
    fn drop(&mut self) {
        for s in [self.old.as_ref(), self.new.as_ref()].into_iter().flatten() {
            if let Some(sample) = &s.sample {
                self.retired.borrow_mut().push(sample.image.clone());
            }
        }
    }
}

impl MediaView {
    pub fn new(old: Option<MediaSide>, new: Option<MediaSide>, file: bool, retired: Rc<RefCell<Vec<Arc<RenderImage>>>>) -> Self {
        Self { old, new, file, source_on: false, retired, stage: (0., 0.), scroll: [(0., 0.); 2], drag: None, stage_origin: Rc::default() }
    }

    pub fn side(&self, i: usize) -> Option<&MediaSide> {
        match i {
            0 if self.file && self.new.is_some() => None,
            0 => self.old.as_ref(),
            _ => self.new.as_ref(),
        }
    }

    pub fn both(&self) -> bool {
        self.side(0).is_some() && self.side(1).is_some()
    }

    pub fn set_file(&mut self, file: bool) {
        self.file = file;
    }

    fn single(&self) -> Option<&'static str> {
        if self.file {
            None
        } else if self.old.is_none() {
            Some("added")
        } else if self.new.is_none() {
            Some("deleted")
        } else {
            None
        }
    }

    /// What's above a font's samples: the padding, the label with two sides, its name.
    fn above_samples(&self) -> f32 {
        PAD + if self.both() { 15.4 + 12. } else { 0. } + 18.2 + 12.
    }

    fn pane_w(&self) -> f32 {
        if self.both() { (self.stage.0 - 2.) / 2. } else { self.stage.0 }
    }

    /// How far a pane scrolls, down and across.
    fn max_scroll(&self, i: usize) -> (f32, f32) {
        let Some(sample) = self.side(i).and_then(|s| s.sample.as_ref()) else { return (0., 0.) };
        ((self.above_samples() + sample.h + PAD - self.stage.1).max(0.), (PAD + sample.w - (self.pane_w() - if sample.content_w < self.pane_w() - 2. * PAD { GUTTER } else { 0. })).max(0.))
    }

    /// What a script checks.
    pub fn dump(&self) -> serde_json::Value {
        let side = |s: Option<&MediaSide>| {
            s.map(|s| serde_json::json!({ "kind": format!("{:?}", s.kind), "codec": s.codec, "info": s.info, "family": s.name.as_ref().map(|n| n.family.clone()), "style": s.name.as_ref().map(|n| n.style.clone()), "sample": s.sample.as_ref().map(|i| (i.w, i.h)), "failed": s.failed, "size": s.size }))
        };
        serde_json::json!({ "hex": self.source_on, "single": self.single(), "file": self.file, "old": side(self.side(0)), "new": side(self.side(1)) })
    }
}

impl GitVolt {
    /// A font pane's samples, for its size and the theme's text.
    pub fn draw_font_samples(&mut self, width: f32, height: f32, cx: &mut Context<Self>) {
        let repo = self.tabs.get(self.active).map_or(0, |t| t.id);
        let rt = self.rt.clone();
        let svg = cx.svg_renderer();
        let Some(diff) = self.tabs.get_mut(self.active).and_then(|t| t.diff.as_mut()) else { return };
        let generation = diff.generation;
        let Some(media) = diff.media.as_mut() else { return };
        media.stage = (width, height);
        let color = Rgba::from(theme::text_normal());
        let (above, pane_w, both) = (media.above_samples(), media.pane_w() - 2. * PAD, media.both());
        let at = (pane_w, height);
        for i in 0..2 {
            let shown = media.side(i).is_some();
            let side = if i == 0 { media.old.as_mut() } else { media.new.as_mut() };
            let Some(side) = side.filter(|_| shown) else { continue };
            let Some(sfnt) = side.sfnt.clone() else { continue };
            if side.drawing || (side.sample_for == at && side.theme == theme::version()) {
                continue;
            }
            side.drawing = true;
            side.sample_for = at;
            let svg = svg.clone();
            let job = rt.spawn_blocking(move || font_sample(&sfnt, pane_w, above, height, &color, &svg));
            let _ = both;
            cx.spawn(async move |this, cx| {
                let made = job.await.ok().flatten();
                this.update(cx, |this, cx| {
                    let Some(media) = this.tabs.iter_mut().find(|t| t.id == repo).and_then(|t| t.diff.as_mut()).filter(|d| d.generation == generation).and_then(|d| d.media.as_mut()) else { return };
                    let retired = media.retired.clone();
                    let side = if i == 0 { media.old.as_mut() } else { media.new.as_mut() };
                    let Some(side) = side else { return };
                    side.drawing = false;
                    side.theme = theme::version();
                    side.failed = made.is_none();
                    if let Some(old) = side.sample.take() {
                        retired.borrow_mut().push(old.image.clone());
                    }
                    side.sample = made.map(Arc::new);
                    cx.notify();
                })
                .ok();
            })
            .detach();
        }
    }

    pub fn media_scroll(&mut self, by: f32, cx: &mut Context<Self>) {
        self.with_diff(cx, |d| {
            if let Some(m) = d.media.as_mut() {
                for i in 0..2 {
                    let max = m.max_scroll(i).0;
                    m.scroll[i].0 = (m.scroll[i].0 + by).clamp(0., max);
                }
            }
        });
    }
}

fn thumb(view: f32, total: f32, at: f32, max: f32) -> (f32, f32) {
    let size = (view * view / total.max(1.)).max(24.);
    (size, (view - size) * (at / max.max(1.)))
}

fn font_pane(i: usize, media: &MediaView, side: &MediaSide, label: Option<&'static str>, cx: &mut Context<GitVolt>) -> AnyElement {
    let name = side.name.as_ref().map_or(" ".to_string(), |n| if n.style.is_empty() { n.family.clone() } else { format!("{} · {}", n.family, n.style) });
    let sample = side.sample.clone();
    let failed = side.failed;
    let loading = !failed && sample.is_none();
    let (sy, sx) = media.scroll[i];
    let (max_y, max_x) = media.max_scroll(i);
    let (sy, sx) = (sy.min(max_y), sx.min(max_x));
    let room = sample.as_ref().map_or(media.pane_w() - 2. * PAD, |s| s.content_w);
    let centre_w = room + 2. * PAD;
    let (view_w, view_h) = (media.pane_w(), media.stage.1);
    let picture = sample.clone();
    div()
        .id(if i == 0 { "media-font-old" } else { "media-font-new" })
        .relative()
        .flex_1()
        .min_w_0()
        .h_full()
        .overflow_hidden()
        .bg(theme::app_bg())
        .on_scroll(cx.listener(move |this, e: &ScrollEvent, window, cx| {
            let delta = e.delta.pixel_delta(window.line_height());
            let (dx, dy) = if e.modifiers.shift { (delta.y.as_f32(), 0.) } else { (delta.x.as_f32(), delta.y.as_f32()) };
            this.with_diff(cx, |d| {
                if let Some(m) = d.media.as_mut() {
                    let (my, mx) = m.max_scroll(i);
                    m.scroll[i] = ((m.scroll[i].0 - dy).clamp(0., my), (m.scroll[i].1 - dx).clamp(0., mx));
                }
            });
            cx.stop_propagation();
        }))
        .on_mouse_down(MouseButton::Left, cx.listener(move |this, e: &MouseDownEvent, _, cx| {
            this.with_diff(cx, |d| {
                d_grab(d, i, e);
            });
        }))
        .on_mouse_move(cx.listener(move |this, e: &MouseMoveEvent, _, cx| {
            let Some(m) = this.tabs.get_mut(this.active).and_then(|t| t.diff.as_mut()).and_then(|d| d.media.as_mut()) else { return };
            let Some((pane, down, from, start)) = m.drag.filter(|_| e.pressed_button == Some(MouseButton::Left)) else { return };
            let (my, mx) = m.max_scroll(pane);
            if down {
                let (t, _) = thumb(view_h, view_h + my, 0., my);
                m.scroll[pane].0 = (start + (e.position.y.as_f32() - from) * my / (view_h - t).max(1.)).clamp(0., my);
            } else {
                let (t, _) = thumb(view_w, view_w + mx, 0., mx);
                m.scroll[pane].1 = (start + (e.position.x.as_f32() - from) * mx / (view_w - t).max(1.)).clamp(0., mx);
            }
            cx.notify();
        }))
        .on_mouse_up(MouseButton::Left, cx.listener(|this, _, _, _| {
            if let Some(m) = this.tabs.get_mut(this.active).and_then(|t| t.diff.as_mut()).and_then(|d| d.media.as_mut()) {
                m.drag = None;
            }
        }))
        .child(
            div()
                .absolute()
                .top_0()
                .bottom_0()
                .left_0()
                .right(px(if room < view_w - 2. * PAD { GUTTER } else { 0. }))
                .overflow_hidden()
                .child(div()
                .absolute()
                .left(px(-sx))
                .top(px(-sy))
                .w(px(centre_w))
                .flex()
                .flex_col()
                .gap(px(12.))
                .p(px(PAD))
                .when_some(label, |d, l| d.child(div().flex().flex_none().justify_center().h(px(15.4)).text_size(px(11.)).line_height(px(15.4)).text_color(theme::text_secondary()).child(l.to_uppercase())))
                .child(div().flex_none().h(px(18.2)).overflow_hidden().whitespace_nowrap().text_color(theme::text_secondary()).child(name))
                .when(failed, |d| d.child(div().flex().justify_center().text_color(theme::text_secondary()).child("This font couldn't be loaded")))
                .when(loading, |d| d.child(div().text_color(theme::text_secondary()).child("Loading…")))
                .when_some(picture, |d, s| {
                    let (w, h, image, lines, grid) = (s.w, s.h, s.image.clone(), s.lines.clone(), s.grid);
                    d.child(
                        canvas(
                            |_, _, _| {},
                            move |b, _, window, cx| {
                                let border = theme::section_border();
                                let (top, cw, ch, per) = grid;
                                for i in 0..94usize {
                                    let (x, y) = ((i % per) as f32 * (cw + 2.), (i / per) as f32 * (ch + 2.) + top);
                                    window.paint_quad(PaintQuad { bounds: Bounds::new(b.origin + point(px(x), px(y)), size(px(cw), px(ch))), corner_radii: Corners::all(px(3.)), background: transparent_black().into(), border_widths: Edges::all(px(1.)), border_color: border, border_style: BorderStyle::Solid });
                                }
                                let at = Bounds::new(b.origin, size(px(w), px(h)));
                                window.paint_image(at, at, Corners::default(), image.clone(), 0, false).ok();
                                let font = window.text_style().font();
                                for (size_px, base) in &lines {
                                    let text: SharedString = format!("{}", *size_px as u32).into();
                                    let run = TextRun { len: text.len(), font: font.clone(), color: theme::text_secondary(), background_color: None, underline: None, strikethrough: None };
                                    let line = window.text_system().shape_line(text, px(10.), &[run], None);
                                    let x = b.origin.x + px(12.) - line.width;
                                    line.paint(point(x, b.origin.y + px(*base - 11.)), px(14.), TextAlign::Left, None, window, cx).ok();
                                }
                            },
                        )
                        .w(px(w))
                        .h(px(h))
                        .flex_none(),
                    )
                })),
        )
        .when(max_y > 0., |d| {
            let (t, at) = thumb(view_h, view_h + max_y, sy, max_y);
            d.child(div().absolute().right(px(2.)).top(px(at)).w(px(6.)).h(px(t)).rounded(px(3.)).bg(theme::scroll_thumb()))
        })
        .when(max_x > 0., |d| {
            let (t, at) = thumb(view_w, view_w + max_x, sx, max_x);
            d.child(div().absolute().bottom(px(2.)).left(px(at)).h(px(6.)).w(px(t)).rounded(px(3.)).bg(theme::scroll_thumb()))
        })
        .into_any_element()
}

/// A press on a font pane's scrollbars.
fn d_grab(d: &mut crate::diff::view::DiffState, i: usize, e: &MouseDownEvent) {
    let Some(m) = d.media.as_mut() else { return };
    let (my, mx) = m.max_scroll(i);
    let (w, h) = (m.pane_w(), m.stage.1);
    let (_, top) = (0., m.stage_origin.get());
    let (x, y) = (e.position.x.as_f32() - top.0 - if i == 1 && m.both() { w + 2. } else { 0. }, e.position.y.as_f32() - top.1);
    if my > 0. && x >= w - 10. {
        m.drag = Some((i, true, e.position.y.as_f32(), m.scroll[i].0));
    } else if mx > 0. && y >= h - 10. {
        m.drag = Some((i, false, e.position.x.as_f32(), m.scroll[i].1));
    }
}

/// The note: what Chromium couldn't play says as the web does; what it could, that this build can't.
pub fn unplayable_note(side_kind: Kind, mime: &str, codec: Option<&str>) -> String {
    let format = video_format(mime, codec);
    match (side_kind, codec) {
        (Kind::Audio, _) => format!("This build can't play audio ({format})"),
        (_, Some("H.264" | "HEVC")) => format!("This video's format ({format}) can't play here"),
        _ => format!("This build can't play video ({format})"),
    }
}

fn player_pane(i: usize, side: &MediaSide, label: Option<&'static str>, open_in: Option<AnyElement>, cx: &mut Context<GitVolt>) -> AnyElement {
    div()
        .flex()
        .flex_col()
        .flex_1()
        .min_w_0()
        .h_full()
        .items_center()
        .justify_center()
        .gap(px(12.))
        .p(px(16.))
        .bg(theme::app_bg())
        .when_some(label, |d, l| d.child(div().h(px(15.4)).text_size(px(11.)).line_height(px(15.4)).text_color(theme::text_secondary()).child(l.to_uppercase())))
        .child(
            div()
                .flex()
                .flex_col()
                .items_center()
                .gap(px(8.))
                .text_color(theme::text_secondary())
                .child(unplayable_note(side.kind, side.mime, side.codec))
                .when_some(side.info.clone(), |d, i| d.child(i))
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap(px(8.))
                        .children(open_in)
                        .child(
                            div()
                                .id(if i == 0 { "media-show-hex-old" } else { "media-show-hex-new" })
                                .px(px(10.))
                                .py(px(3.))
                                .rounded(px(3.))
                                .bg(theme::blue())
                                .text_color(theme::text_on_accent())
                                .on_click(cx.listener(|this, _, _, cx| this.toggle_media_hex(cx)))
                                .child("Show hex"),
                        ),
                ),
        )
        .into_any_element()
}

impl GitVolt {
    pub fn toggle_media_hex(&mut self, cx: &mut Context<Self>) {
        let mut load = false;
        self.with_diff(cx, |d| {
            if let Some(m) = d.media.as_mut() {
                m.source_on = !m.source_on;
                load = m.source_on && d.hex.is_none();
            }
            d.layout_again();
        });
        if load {
            self.read_hex(cx);
        }
    }
}

/// Audio, video or a font, in the diff panel's body.
pub fn media_body(media: &MediaView, source: Option<AnyElement>, open_in: Option<AnyElement>, cx: &mut Context<GitVolt>) -> AnyElement {
    let both = media.both();
    let sizes: Vec<String> = [media.side(0), media.side(1)].into_iter().flatten().map(|s| crate::format::format_bytes(Some(s.size))).collect();
    let single = media.single();
    let meta_text = format!("{}{}", sizes.join(" → "), single.map_or(String::new(), |s| format!(" ({s})")));
    let source_on = media.source_on;
    let shown = div().flex().flex_none().items_center().gap(px(3.5)).text_color(theme::text_disabled()).child(crate::diff::image::arrowed(sizes)).when_some(single, |d, s| d.child(format!("({s})")));
    let head = div()
        .flex()
        .flex_none()
        .h(px(TOOLBAR_H))
        .items_center()
        .gap(px(10.))
        .px(px(8.))
        .border_b_1()
        .border_color(theme::section_border())
        .child(div().id("media-hex").flex_none().px(px(8.)).py(px(2.)).border_1().border_color(theme::section_border()).rounded(px(4.)).text_color(if source_on { theme::text_selected() } else { theme::text_secondary() }).when(source_on, |d| d.bg(theme::selected_row())).on_click(cx.listener(|this, _, _, cx| this.toggle_media_hex(cx))).child("Hex"))
        .child(tip(meta_text, div().id("media-meta").ml(px(6.)).child(shown)));
    let stage: AnyElement = match source {
        Some(s) if source_on => div().flex().flex_col().flex_1().min_h_0().child(s).into_any_element(),
        _ => {
            let mut open_in = open_in;
            let origin = media.stage_origin.clone();
            let mut row = div()
                .id("media-stage")
                .relative()
                .flex()
                .gap(px(2.))
                .flex_1()
                .min_h_0()
                .child(canvas(move |b, _, _| origin.set((b.origin.x.as_f32(), b.origin.y.as_f32())), |_, _, _, _| {}).absolute().size_full());
            let label = |text: &'static str| both.then_some(text);
            for (i, text) in [(0, "Old"), (1, "New")] {
                let Some(side) = media.side(i) else { continue };
                let pane = if side.kind == Kind::Font {
                    font_pane(i, media, side, label(text), cx)
                } else {
                    // Open in opens the new side, or the one side there is.
                    let here = if i == 1 || !both { open_in.take() } else { None };
                    player_pane(i, side, label(text), here, cx)
                };
                row = row.child(pane);
            }
            if both {
                row = row.child(div().absolute().top_0().bottom_0().left(relative(0.5)).w(px(1.)).bg(theme::section_border()));
            }
            row.into_any_element()
        }
    };
    div().flex().flex_col().flex_1().min_w_0().min_h_0().child(head).child(stage).into_any_element()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::prelude::v1::test;

    #[test]
    fn a_file_is_told_by_its_extension() {
        assert_eq!(media_of("a/b.MP3"), Some((Kind::Audio, "audio/mpeg")));
        assert_eq!(media_of("clip.webm"), Some((Kind::Video, "video/webm")));
        assert_eq!(media_of("f.woff2"), Some((Kind::Font, "font/woff2")));
        assert_eq!(media_of("a.png"), None);
        assert_eq!(media_of("noext"), None);
    }

    #[test]
    fn an_mp4_names_its_codec() {
        let mut b = vec![0, 0, 0, 20];
        b.extend_from_slice(b"ftypisom");
        b.extend_from_slice(&[0; 8]);
        b.extend_from_slice(b"....avc1....");
        assert_eq!(video_codec(&b), Some("H.264"));
        assert_eq!(video_codec(b"not a video at all"), None);
        assert_eq!(video_format("video/mp4", Some("H.264")), "H.264");
        assert_eq!(video_format("audio/flac", None), "FLAC");
    }

    #[test]
    fn a_wav_says_its_length_rate_and_channels() {
        let mut b = b"RIFF\0\0\0\0WAVEfmt ".to_vec();
        b.extend_from_slice(&16u32.to_le_bytes());
        b.extend_from_slice(&1u16.to_le_bytes());
        b.extend_from_slice(&1u16.to_le_bytes());
        b.extend_from_slice(&22050u32.to_le_bytes());
        b.extend_from_slice(&44100u32.to_le_bytes());
        b.extend_from_slice(&2u16.to_le_bytes());
        b.extend_from_slice(&16u16.to_le_bytes());
        b.extend_from_slice(b"data");
        b.extend_from_slice(&44100u32.to_le_bytes());
        assert_eq!(describe(&b).as_deref(), Some("1.0 s · 22050 Hz · mono"));
        assert_eq!(describe(b"RIFF"), None);
    }

    #[test]
    fn a_table_of_names_gives_the_family_and_style() {
        // format, count, strings at; then records (platform 3, en) for ids 1 and 2.
        let (fam, sty): (Vec<u8>, Vec<u8>) = ("Quay".encode_utf16().flat_map(|u| u.to_be_bytes()).collect(), "Bold".encode_utf16().flat_map(|u| u.to_be_bytes()).collect());
        let mut t = Vec::new();
        for v in [0u16, 2, 30] {
            t.extend_from_slice(&v.to_be_bytes());
        }
        for (id, len, off) in [(1u16, fam.len() as u16, 0u16), (2, sty.len() as u16, fam.len() as u16)] {
            for v in [3u16, 1, 0x409, id, len, off] {
                t.extend_from_slice(&v.to_be_bytes());
            }
        }
        t.extend_from_slice(&fam);
        t.extend_from_slice(&sty);
        assert_eq!(read_names(&t), Some(FontName { family: "Quay".into(), style: "Bold".into() }));
    }

    #[test]
    fn the_bundled_font_gives_its_name_and_a_woff_of_it_the_same() {
        let ttf = include_bytes!("../assets/fonts/JetBrainsMono-Latin-400.ttf");
        let name = font_name(ttf).unwrap();
        assert!(name.family.contains("JetBrains"), "{name:?}");
        assert!(ttf_parser::Face::parse(ttf, 0).is_ok());
        // A hostile WOFF: a table claiming to unpack to gigabytes is refused before it's unpacked.
        let mut w = b"wOFF\0\x01\0\0".to_vec();
        w.resize(44, 0);
        w[12] = 0;
        w[13] = 1;
        w.extend_from_slice(b"name");
        w.extend_from_slice(&64u32.to_be_bytes());
        w.extend_from_slice(&4u32.to_be_bytes());
        w.extend_from_slice(&u32::MAX.to_be_bytes());
        w.extend_from_slice(&0u32.to_be_bytes());
        w.extend_from_slice(&[1, 2, 3, 4]);
        assert!(sfnt_of(&w).is_none());
        assert!(sfnt_of(b"wOF2 and so on").is_none(), "WOFF2 has no name here");
    }

    #[test]
    fn the_samples_lay_out_as_the_web_measures_them() {
        // JetBrains Mono's metrics, in a 536 px pane: the reference's rows and cells.
        let (lines, top, (_, cell_w, cell_h, per), h) = sample_layout(1.02, 0.3, 536.);
        assert_eq!(lines.len(), 5);
        assert!((top - 251.2).abs() < 0.01, "{top}");
        assert_eq!(per, 10);
        assert!((cell_w - 51.8).abs() < 0.01 && (cell_h - 41.6).abs() < 0.01);
        assert!((h - 684.8).abs() < 1., "{h}");
    }

    #[test]
    fn a_fonts_outlines_stop_at_their_bound() {
        let ttf = include_bytes!("../assets/fonts/JetBrainsMono-Latin-400.ttf");
        let face = ttf_parser::Face::parse(ttf, 0).unwrap();
        let mut d = String::new();
        let w = text_path(&face, PANGRAM, 12., 0., 12., &mut d);
        assert!(w > 0. && !d.is_empty());
        let mut full = "x".repeat(MAX_OUTLINE_BYTES);
        let same = text_path(&face, PANGRAM, 12., 0., 12., &mut full);
        assert_eq!(full.len(), MAX_OUTLINE_BYTES, "no more outlines");
        assert_eq!(same, w, "the width's still known");
    }

    #[test]
    fn what_plays_in_the_web_says_this_build_cant() {
        assert_eq!(unplayable_note(Kind::Video, "video/mp4", Some("H.264")), "This video's format (H.264) can't play here");
        assert_eq!(unplayable_note(Kind::Video, "video/webm", None), "This build can't play video (WebM)");
        assert_eq!(unplayable_note(Kind::Audio, "audio/wav", None), "This build can't play audio (WAV)");
    }

    #[test]
    fn a_zip_bomb_in_a_table_stops_at_its_limit() {
        use std::io::Write;
        let mut z = flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::best());
        z.write_all(&vec![0u8; 8 << 20]).unwrap();
        let packed = z.finish().unwrap();
        assert!(inflate(&packed, 1 << 20).is_none());
        assert_eq!(inflate(&packed, 8 << 20).map(|v| v.len()), Some(8 << 20));
    }
}
