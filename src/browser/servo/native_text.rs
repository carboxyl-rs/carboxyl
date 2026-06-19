use std::collections::HashMap;
use std::sync::mpsc;

use log::warn;
use servo::{JSValue, JavaScriptEvaluationError, WebView};

use crate::output::TextNode;

use super::events::RuntimeEvent;

// ---------------------------------------------------------------------------
// JavaScript source
// ---------------------------------------------------------------------------

/// Injected once per page load to make all text transparent in Servo's pixel
/// render. Layout is unaffected — only paint color changes — so `EXTRACTION_SCRIPT`
/// still returns accurate positions.
pub const SUPPRESS_TEXT_SCRIPT: &str = include_str!("suppress.js");

/// Evaluates on the page and returns an Array of Objects with fields:
/// `t` (text), `x`, `y`, `w`, `h` (viewport-relative CSS px, content-box
/// origin), `c` (CSS color read with suppression temporarily disabled).
pub const EXTRACTION_SCRIPT: &str = include_str!("extract.js");

// ---------------------------------------------------------------------------
// JS result parsing
// ---------------------------------------------------------------------------

pub fn parse_js_nodes(value: &JSValue) -> Vec<TextNode> {
    let JSValue::Array(items) = value else {
        return vec![];
    };

    items
        .iter()
        .filter_map(|item| {
            let JSValue::Object(map) = item else {
                return None;
            };

            let text = str_field(map, "t")?.trim().to_owned();
            if text.is_empty() {
                return None;
            }

            let x = f32_field(map, "x")?;
            let y = f32_field(map, "y")?;
            let w = f32_field(map, "w")?;
            let h = f32_field(map, "h")?;

            if w <= 0.0 || h <= 0.0 || x < 0.0 || y < 0.0 {
                return None;
            }

            let color = str_field(map, "c")
                .and_then(parse_css_color)
                .unwrap_or(ratatui::style::Color::Reset);

            Some(TextNode { text, x, y, width: w, height: h, color })
        })
        .collect()
}

fn str_field<'a>(map: &'a HashMap<String, JSValue>, key: &str) -> Option<&'a str> {
    match map.get(key)? {
        JSValue::String(s) => Some(s.as_str()),
        _ => None,
    }
}

fn f32_field(map: &HashMap<String, JSValue>, key: &str) -> Option<f32> {
    match map.get(key)? {
        JSValue::Number(n) => Some(*n as f32),
        _ => None,
    }
}

/// Parse CSS `rgb(r, g, b)` or `rgba(r, g, b, a)`. Alpha is ignored.
fn parse_css_color(s: &str) -> Option<ratatui::style::Color> {
    let inner = s
        .trim()
        .strip_prefix("rgba(")
        .or_else(|| s.trim().strip_prefix("rgb("))?
        .strip_suffix(')')?;

    let mut parts = inner.split(',').map(|p| p.trim());
    let r: u8 = parts.next()?.parse().ok()?;
    let g: u8 = parts.next()?.parse().ok()?;
    let b: u8 = parts.next()?.parse().ok()?;

    Some(ratatui::style::Color::Rgb(r, g, b))
}

// ---------------------------------------------------------------------------
// WebView integration
// ---------------------------------------------------------------------------

pub fn suppress(webview: &WebView) {
    webview.evaluate_javascript(SUPPRESS_TEXT_SCRIPT, |result| {
        if let Err(e) = result
            && !matches!(e, JavaScriptEvaluationError::WebViewNotReady)
        {
            warn!("text suppression failed: {e:?}");
        }
    });
}

pub fn extract(webview: &WebView, event_tx: mpsc::SyncSender<RuntimeEvent>) {
    webview.evaluate_javascript(EXTRACTION_SCRIPT, move |result| match result {
        Ok(value) => {
            let nodes = parse_js_nodes(&value);
            if !nodes.is_empty() {
                let _ = event_tx.try_send(RuntimeEvent::TextNodes(nodes));
            }
        }
        Err(e) => {
            if !matches!(e, JavaScriptEvaluationError::WebViewNotReady) {
                warn!("text extraction failed: {e:?}");
            }
        }
    });
}
