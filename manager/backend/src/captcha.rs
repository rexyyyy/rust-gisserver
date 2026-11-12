//! 登录验证码：自绘 SVG（不引入图像库），内存保存 60 秒。
use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use base64::{engine::general_purpose::STANDARD, Engine as _};
use rand::Rng;
use serde::Serialize;

#[derive(Debug)]
struct Entry {
    code: String,
    at: Instant,
}

#[derive(Debug, Default)]
pub struct CaptchaStore {
    inner: Mutex<HashMap<String, Entry>>,
}

const TTL: Duration = Duration::from_secs(60);
const CHARS: &[u8] = b"ABCDEFGHJKLMNPQRSTUVWXY345678";

impl CaptchaStore {
    /// 生成一张验证码，返回 `(uuid, data URL)`。
    pub fn issue(&self) -> (String, String) {
        let mut rng = rand::thread_rng();
        let code: String = (0..4)
            .map(|_| CHARS[rng.gen_range(0..CHARS.len())] as char)
            .collect();
        let uuid = uuid::Uuid::new_v4().simple().to_string();
        let svg = render(&code, &mut rng);
        let url = format!("data:image/svg+xml;base64,{}", STANDARD.encode(svg));
        self.gc();
        self.inner.lock().unwrap().insert(
            uuid.clone(),
            Entry { code: code.to_ascii_lowercase(), at: Instant::now() },
        );
        (uuid, url)
    }

    /// 校验并立即失效（一次性）。忽略大小写。
    pub fn check(&self, uuid: &str, input: &str) -> bool {
        let mut m = self.inner.lock().unwrap();
        let entry = match m.get(uuid) {
            Some(e) => e,
            None => return false,
        };
        if entry.at.elapsed() > TTL {
            m.remove(uuid);
            return false;
        }
        let ok = entry.code == input.trim().to_ascii_lowercase();
        m.remove(uuid);
        ok
    }

    fn gc(&self) {
        self.inner
            .lock()
            .unwrap()
            .retain(|_, e| e.at.elapsed() <= TTL);
    }
}

#[derive(Debug, Serialize)]
pub struct CaptchaVO {
    pub captcha_enabled: bool,
    pub img: String,
    pub uuid: String,
}

/// 画一张 120×48 的 SVG：随机底噪 + 四个错位字符。
fn render(code: &str, rng: &mut impl Rng) -> String {
    let mut svg = String::from(
        r##"<svg xmlns="http://www.w3.org/2000/svg" width="120" height="48" viewBox="0 0 120 48">"##,
    );
    svg.push_str(r##"<rect width="120" height="48" fill="#f4f6f8"/>"##);
    for _ in 0..5 {
        let x1 = rng.gen_range(0..120);
        let y1 = rng.gen_range(0..48);
        let x2 = rng.gen_range(0..120);
        let y2 = rng.gen_range(0..48);
        let c = rng.gen_range(120..210);
        svg.push_str(&format!(
            r##"<line x1="{x1}" y1="{y1}" x2="{x2}" y2="{y2}" stroke="rgb({c},{c},{c})" stroke-width="1"/>"##
        ));
    }
    for (i, ch) in code.chars().enumerate() {
        let x: i32 = 16 + (i as i32) * 24 + rng.gen_range(-3..3);
        let y: i32 = 32 + rng.gen_range(-4..4);
        let rot = rng.gen_range(-18..18);
        let fill = match rng.gen_range(0..4) {
            0 => "#3b82f6",
            1 => "#ef4444",
            2 => "#10b981",
            _ => "#1f2937",
        };
        svg.push_str(&format!(
            r##"<text x="{x}" y="{y}" font-family="Verdana" font-size="26" fill="{fill}" transform="rotate({rot} {x} {y})">{ch}</text>"##
        ));
    }
    svg.push_str("</svg>");
    svg
}
