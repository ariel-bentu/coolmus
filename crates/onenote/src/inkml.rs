//! InkML ⇄ strokes. Supports the subset OneNote reads and writes, plus the
//! InkML difference encodings (`!`, `'`, `"`) so foreign ink also parses.

use std::collections::HashMap;
use std::fmt::Write as _;

use quick_xml::events::{BytesStart, Event};
use quick_xml::Reader;
use sketch_core::{Color, Pen, PenStyle, Point, Stroke};

use crate::{Error, Result};

/// Himetric units (1/1000 cm) per page unit (1/96 inch).
pub const HIMETRIC_PER_PX: f32 = 2540.0 / 96.0;
const PRESSURE_MAX: f32 = 32767.0;

fn unit_to_px(units: &str) -> f32 {
    match units.to_ascii_lowercase().as_str() {
        "cm" => 96.0 / 2.54,
        "mm" => 96.0 / 25.4,
        "in" => 96.0,
        "pt" => 96.0 / 72.0,
        "px" | "dev" => 1.0,
        _ => 1.0 / HIMETRIC_PER_PX, // himetric (OneNote's default)
    }
}

#[derive(Debug, Clone)]
struct Channel {
    name: String,
    max: Option<f32>,
    to_px: f32,
}

#[derive(Debug, Clone)]
struct Brush {
    width_px: f32,
    color: Color,
    transparency: u8,
    highlighter: bool,
}

impl Default for Brush {
    fn default() -> Self {
        Self { width_px: 2.0, color: Color::BLACK, transparency: 0, highlighter: false }
    }
}

fn default_channels() -> Vec<Channel> {
    let him = 1.0 / HIMETRIC_PER_PX;
    vec![
        Channel { name: "X".into(), max: None, to_px: him },
        Channel { name: "Y".into(), max: None, to_px: him },
    ]
}

fn attr(e: &BytesStart, name: &str) -> Option<String> {
    e.attributes().flatten().find_map(|a| {
        (a.key.local_name().as_ref() == name)
            .then(|| a.normalized_value(quick_xml::XmlVersion::Implicit1_0).map(|v| v.into_owned()).unwrap_or_else(|_| a.value.into_owned()))
    })
}

fn strip_ref(r: &str) -> String {
    r.trim_start_matches('#').to_string()
}

/// Parses InkML into strokes in page units.
pub fn parse(xml: &str) -> Result<Vec<Stroke>> {
    let mut reader = Reader::from_str(xml);
    let mut contexts: HashMap<String, Vec<Channel>> = HashMap::new();
    let mut brushes: HashMap<String, Brush> = HashMap::new();

    let mut cur_context: Option<(String, Vec<Channel>)> = None;
    let mut cur_brush: Option<(String, Brush)> = None;
    let mut cur_trace: Option<(Option<String>, Option<String>, String)> = None;
    let mut last_context = String::new();
    let mut last_brush = String::new();
    let mut strokes = Vec::new();

    loop {
        let ev = reader.read_event().map_err(|e| Error::Parse(format!("InkML: {e}")))?;
        let (e, empty) = match &ev {
            Event::Start(e) => (Some(e.clone()), false),
            Event::Empty(e) => (Some(e.clone()), true),
            _ => (None, false),
        };
        if let Some(e) = e {
            match e.local_name().as_ref() {
                "context" => {
                    let id = attr(&e, "id").unwrap_or_default();
                    cur_context = Some((id, Vec::new()));
                }
                "channel" => {
                    if let Some((_, chans)) = cur_context.as_mut() {
                        chans.push(Channel {
                            name: attr(&e, "name").unwrap_or_default().to_ascii_uppercase(),
                            max: attr(&e, "max").and_then(|v| v.parse().ok()),
                            to_px: unit_to_px(&attr(&e, "units").unwrap_or_default()),
                        });
                    }
                }
                "brush" => {
                    cur_brush = Some((attr(&e, "id").unwrap_or_default(), Brush::default()));
                }
                "brushProperty" => {
                    if let Some((_, b)) = cur_brush.as_mut() {
                        let name = attr(&e, "name").unwrap_or_default();
                        let value = attr(&e, "value").unwrap_or_default();
                        match name.as_str() {
                            "width" => {
                                let units = attr(&e, "units").unwrap_or_else(|| "himetric".into());
                                if let Ok(v) = value.parse::<f32>() {
                                    b.width_px = v * unit_to_px(&units);
                                }
                            }
                            "color" => b.color = Color::from_hex(&value).unwrap_or(Color::BLACK),
                            "transparency" => b.transparency = value.parse::<u16>().unwrap_or(0).min(255) as u8,
                            "rasterOp" => b.highlighter |= value.eq_ignore_ascii_case("maskPen"),
                            "tip" => b.highlighter |= value.eq_ignore_ascii_case("rectangle"),
                            _ => {}
                        }
                    }
                }
                "trace" => {
                    let t = (attr(&e, "contextRef").map(|r| strip_ref(&r)), attr(&e, "brushRef").map(|r| strip_ref(&r)), String::new());
                    if empty {
                        // An empty trace has no points.
                    } else {
                        cur_trace = Some(t);
                    }
                }
                _ => {}
            }
            if empty {
                end_element(e.local_name().as_ref(), &mut cur_context, &mut cur_brush, &mut contexts, &mut brushes);
            }
            continue;
        }
        match ev {
            Event::Text(t) => {
                if let Some((_, _, text)) = cur_trace.as_mut() {
                    text.push_str(&t);
                }
            }
            Event::End(e) => {
                if e.local_name().as_ref() == "trace" {
                    if let Some((ctx, br, text)) = cur_trace.take() {
                        let ctx = ctx.unwrap_or_else(|| last_context.clone());
                        let br = br.unwrap_or_else(|| last_brush.clone());
                        let chans = contexts.get(&ctx).cloned().unwrap_or_else(default_channels);
                        let brush = brushes.get(&br).cloned().unwrap_or_default();
                        if let Some(s) = build_stroke(&chans, &brush, &text) {
                            strokes.push(s);
                        }
                        last_context = ctx;
                        last_brush = br;
                    }
                } else {
                    end_element(e.local_name().as_ref(), &mut cur_context, &mut cur_brush, &mut contexts, &mut brushes);
                }
            }
            Event::Eof => break,
            _ => {}
        }
    }
    Ok(strokes)
}

fn end_element(
    name: &str,
    cur_context: &mut Option<(String, Vec<Channel>)>,
    cur_brush: &mut Option<(String, Brush)>,
    contexts: &mut HashMap<String, Vec<Channel>>,
    brushes: &mut HashMap<String, Brush>,
) {
    match name {
        "context" => {
            if let Some((id, chans)) = cur_context.take() {
                contexts.insert(id, if chans.is_empty() { default_channels() } else { chans });
            }
        }
        "brush" => {
            if let Some((id, b)) = cur_brush.take() {
                brushes.insert(id, b);
            }
        }
        _ => {}
    }
}

#[derive(Clone, Copy, PartialEq)]
enum Mode {
    Explicit,
    First,
    Second,
}

/// Decodes InkML trace text into rows of absolute channel values.
pub fn decode_trace(text: &str, channels: usize) -> Vec<Vec<f32>> {
    let mut modes = vec![Mode::Explicit; channels];
    let mut value = vec![0f32; channels];
    let mut velocity = vec![0f32; channels];
    let mut rows = Vec::new();
    for point in text.split(',') {
        let tokens = tokenize(point);
        if tokens.is_empty() {
            continue;
        }
        for (c, (prefix, x)) in tokens.into_iter().take(channels).enumerate() {
            if let Some(m) = prefix {
                modes[c] = m;
            }
            match modes[c] {
                Mode::Explicit => {
                    velocity[c] = x - value[c];
                    value[c] = x;
                }
                Mode::First => {
                    velocity[c] = x;
                    value[c] += x;
                }
                Mode::Second => {
                    velocity[c] += x;
                    value[c] += velocity[c];
                }
            }
        }
        rows.push(value.clone());
    }
    rows
}

fn tokenize(s: &str) -> Vec<(Option<Mode>, f32)> {
    let mut out = Vec::new();
    let mut prefix = None;
    let mut num = String::new();
    let flush = |num: &mut String, prefix: &mut Option<Mode>, out: &mut Vec<(Option<Mode>, f32)>| {
        if !num.is_empty() {
            if let Ok(v) = num.parse::<f32>() {
                out.push((prefix.take(), v));
            }
            num.clear();
        }
    };
    for c in s.chars() {
        match c {
            '!' | '\'' | '"' => {
                flush(&mut num, &mut prefix, &mut out);
                prefix = Some(match c {
                    '!' => Mode::Explicit,
                    '\'' => Mode::First,
                    _ => Mode::Second,
                });
            }
            '-' | '+' if !num.is_empty() => {
                flush(&mut num, &mut prefix, &mut out);
                num.push(c);
            }
            c if c.is_whitespace() => flush(&mut num, &mut prefix, &mut out),
            c => num.push(c),
        }
    }
    flush(&mut num, &mut prefix, &mut out);
    out
}

fn build_stroke(chans: &[Channel], brush: &Brush, text: &str) -> Option<Stroke> {
    let idx = |n: &str| chans.iter().position(|c| c.name == n);
    let (xi, yi) = (idx("X")?, idx("Y")?);
    let fi = idx("F");
    let rows = decode_trace(text, chans.len());
    if rows.is_empty() {
        return None;
    }
    let mut alpha = 255u16.saturating_sub(brush.transparency as u16) as u8;
    if brush.highlighter {
        alpha = alpha.min(90);
    }
    let pen = Pen {
        color: brush.color.with_alpha(if brush.highlighter { 255 } else { alpha }),
        width: brush.width_px.max(0.5),
        style: if brush.highlighter { PenStyle::Highlighter } else { PenStyle::Solid },
        pressure_sensitive: fi.is_some(),
    };
    let mut s = Stroke::new(pen);
    s.points = rows
        .iter()
        .map(|r| {
            let pressure = fi
                .map(|f| (r[f] / chans[f].max.unwrap_or(PRESSURE_MAX)).clamp(0.0, 1.0))
                .unwrap_or(1.0);
            Point::new(r[xi] * chans[xi].to_px, r[yi] * chans[yi].to_px, pressure)
        })
        .collect();
    Some(s)
}

/// Serializes strokes as OneNote-compatible InkML. Dashed and dotted strokes
/// are emitted as one trace per dash/dot so they look right in OneNote.
pub fn write(strokes: &[Stroke]) -> String {
    let mut brushes: Vec<(Color, u32, PenStyle)> = Vec::new();
    let mut traces = String::new();
    let mut trace_id = 0;
    for s in strokes {
        let color = s.pen.effective_color();
        let width = (s.pen.width * HIMETRIC_PER_PX).round().max(1.0) as u32;
        let key = (color, width, s.pen.style);
        let bi = brushes.iter().position(|b| *b == key).unwrap_or_else(|| {
            brushes.push(key);
            brushes.len() - 1
        });
        let runs: Vec<Vec<Point>> = match s.pen.style {
            PenStyle::Dashed | PenStyle::Dotted => sketch_render::tessellate::pieces(s)
                .into_iter()
                .map(|piece| {
                    let mut pts: Vec<Point> = piece
                        .pts
                        .iter()
                        .zip(&piece.radii)
                        .map(|(p, r)| Point::new(p[0], p[1], (2.0 * r / s.pen.width).clamp(0.0, 1.0)))
                        .collect();
                    if pts.len() == 1 {
                        // A dot: OneNote ignores single-point traces, so add a tiny segment.
                        let p = pts[0];
                        pts.push(Point::new(p.x + 0.3, p.y, p.pressure));
                    }
                    pts
                })
                .collect(),
            _ => vec![s.points.clone()],
        };
        for run in runs.into_iter().filter(|r| !r.is_empty()) {
            let pts: Vec<String> = run
                .iter()
                .map(|p| {
                    format!(
                        "{} {} {}",
                        (p.x * HIMETRIC_PER_PX).round() as i64,
                        (p.y * HIMETRIC_PER_PX).round() as i64,
                        (p.pressure.clamp(0.0, 1.0) * PRESSURE_MAX).round() as i64
                    )
                })
                .collect();
            let _ = writeln!(
                traces,
                "    <inkml:trace xml:id=\"t{trace_id}\" contextRef=\"#ctxCoordinatesWithPressure\" brushRef=\"#br{bi}\">{}</inkml:trace>",
                pts.join(", ")
            );
            trace_id += 1;
        }
    }
    let mut out = String::from(
        "<?xml version=\"1.0\" encoding=\"utf-8\"?>\n<inkml:ink xmlns:inkml=\"http://www.w3.org/2003/InkML\">\n  <inkml:definitions>\n    <inkml:context xml:id=\"ctxCoordinatesWithPressure\">\n      <inkml:inkSource xml:id=\"inkSrcCoordinatesWithPressure\">\n        <inkml:traceFormat>\n          <inkml:channel name=\"X\" type=\"integer\" max=\"32767\" units=\"himetric\"/>\n          <inkml:channel name=\"Y\" type=\"integer\" max=\"32767\" units=\"himetric\"/>\n          <inkml:channel name=\"F\" type=\"integer\" max=\"32767\" units=\"dev\"/>\n        </inkml:traceFormat>\n        <inkml:channelProperties>\n          <inkml:channelProperty channel=\"X\" name=\"resolution\" value=\"1\" units=\"1/himetric\"/>\n          <inkml:channelProperty channel=\"Y\" name=\"resolution\" value=\"1\" units=\"1/himetric\"/>\n          <inkml:channelProperty channel=\"F\" name=\"resolution\" value=\"1\" units=\"1/dev\"/>\n        </inkml:channelProperties>\n      </inkml:inkSource>\n    </inkml:context>\n",
    );
    for (i, (color, width, style)) in brushes.iter().enumerate() {
        let hl = *style == PenStyle::Highlighter;
        let transparency = 255 - color.a as u16;
        let _ = write!(
            out,
            "    <inkml:brush xml:id=\"br{i}\">\n      <inkml:brushProperty name=\"width\" value=\"{width}\" units=\"himetric\"/>\n      <inkml:brushProperty name=\"height\" value=\"{width}\" units=\"himetric\"/>\n      <inkml:brushProperty name=\"color\" value=\"{}\"/>\n      <inkml:brushProperty name=\"transparency\" value=\"{transparency}\"/>\n      <inkml:brushProperty name=\"tip\" value=\"{}\"/>\n      <inkml:brushProperty name=\"rasterOp\" value=\"{}\"/>\n      <inkml:brushProperty name=\"ignorePressure\" value=\"{}\"/>\n    </inkml:brush>\n",
            color.to_hex().to_uppercase(),
            if hl { "rectangle" } else { "ellipse" },
            if hl { "maskPen" } else { "copyPen" },
            hl,
        );
    }
    out.push_str("  </inkml:definitions>\n  <inkml:traceGroup>\n");
    out.push_str(&traces);
    out.push_str("  </inkml:traceGroup>\n</inkml:ink>\n");
    out
}

/// A cheap signature of ink content, used to detect whether ink on a OneNote
/// page was changed by someone else since Sketcher wrote it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Fingerprint {
    pub traces: usize,
    pub points: usize,
}

pub fn fingerprint(strokes: &[Stroke]) -> Fingerprint {
    Fingerprint { traces: strokes.len(), points: strokes.iter().map(|s| s.points.len()).sum() }
}

#[cfg(test)]
mod tests {
    use super::*;

    const ONENOTE_SAMPLE: &str = r##"<?xml version="1.0" encoding="utf-8"?>
<inkml:ink xmlns:inkml="http://www.w3.org/2003/InkML">
  <inkml:definitions>
    <inkml:context xml:id="ctxCoordinatesWithPressure">
      <inkml:inkSource>
        <inkml:traceFormat>
          <inkml:channel name="X" type="integer" max="32767" units="himetric"/>
          <inkml:channel name="Y" type="integer" max="32767" units="himetric"/>
          <inkml:channel name="F" type="integer" max="32767" units="dev"/>
        </inkml:traceFormat>
      </inkml:inkSource>
    </inkml:context>
    <inkml:brush xml:id="br0">
      <inkml:brushProperty name="width" value="100" units="himetric"/>
      <inkml:brushProperty name="color" value="#0000FF"/>
      <inkml:brushProperty name="transparency" value="0"/>
    </inkml:brush>
  </inkml:definitions>
  <inkml:traceGroup>
    <inkml:trace contextRef="#ctxCoordinatesWithPressure" brushRef="#br0">
      1423 7569 3456, 1468 7288 7040
    </inkml:trace>
  </inkml:traceGroup>
</inkml:ink>"##;

    #[test]
    fn parses_onenote_sample() {
        let strokes = parse(ONENOTE_SAMPLE).unwrap();
        assert_eq!(strokes.len(), 1);
        let s = &strokes[0];
        assert_eq!(s.pen.color, Color::rgb(0, 0, 255));
        assert!((s.pen.width - 100.0 / HIMETRIC_PER_PX).abs() < 1e-3);
        assert_eq!(s.points.len(), 2);
        assert!((s.points[0].x - 1423.0 / HIMETRIC_PER_PX).abs() < 1e-3);
        assert!((s.points[1].pressure - 7040.0 / 32767.0).abs() < 1e-4);
    }

    #[test]
    fn decodes_difference_encoding() {
        // Explicit, then first differences, then second differences.
        let rows = decode_trace("10 20, '1 '2, \"1 \"0, 0 0", 2);
        assert_eq!(rows, vec![vec![10.0, 20.0], vec![11.0, 22.0], vec![13.0, 24.0], vec![15.0, 26.0]]);
        // Packed tokens: "'5-3" is two values.
        assert_eq!(decode_trace("0 0,'5-3", 2)[1], vec![5.0, -3.0]);
    }

    #[test]
    fn write_then_parse_roundtrip() {
        let mut a = Stroke::new(Pen { color: Color::rgb(200, 10, 10), width: 4.0, ..Pen::default() });
        a.points = vec![Point::new(10.0, 10.0, 0.5), Point::new(50.0, 40.0, 1.0)];
        let mut hl = Stroke::new(Pen { style: PenStyle::Highlighter, width: 16.0, ..Pen::default() });
        hl.points = vec![Point::new(0.0, 100.0, 1.0), Point::new(200.0, 100.0, 1.0)];
        let xml = write(&[a.clone(), hl]);
        let back = parse(&xml).unwrap();
        assert_eq!(back.len(), 2);
        assert_eq!(back[0].pen.color, a.pen.color);
        assert!((back[0].points[1].x - 50.0).abs() < 0.05);
        assert!((back[0].points[0].pressure - 0.5).abs() < 0.001);
        assert_eq!(back[1].pen.style, PenStyle::Highlighter);
    }
}
