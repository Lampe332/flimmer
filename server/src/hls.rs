//! Minimal HLS playlist parsing and URI rewriting.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};
use url::Url;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Variant {
    pub width: Option<u32>,
    pub height: Option<u32>,
    pub bandwidth: Option<u64>,
    pub codecs: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Track {
    pub lang: Option<String>,
    pub name: Option<String>,
}

pub struct MasterVariant {
    pub info: Variant,
    /// Playlist URI as written in the master.
    pub uri: String,
    audio_group: Option<String>,
}

/// An `EXT-X-MEDIA TYPE=AUDIO` entry that has its own playlist (audio not muxed into the video).
struct AudioRendition {
    group: String,
    lang: Option<String>,
    default: bool,
    uri: String,
}

pub struct Master {
    /// Sorted best first.
    pub variants: Vec<MasterVariant>,
    pub audio: Vec<Track>,
    pub subtitles: Vec<Track>,
    renditions: Vec<AudioRendition>,
}

impl Master {
    /// Playlist URI of the separate audio that plays with `variant`: the rendition in
    /// `lang` ("de" / "en", anything else = no preference), else the default, else the first.
    pub fn audio_uri(&self, variant: &MasterVariant, lang: &str) -> Option<&str> {
        let group = variant.audio_group.as_deref()?;
        let codes: &[&str] = match lang {
            "de" => &["de", "ger", "deu"],
            "en" => &["en", "eng"],
            _ => &[],
        };
        let in_group = || self.renditions.iter().filter(|r| r.group == group);
        let wanted = |r: &&AudioRendition| {
            let l = r.lang.as_deref().unwrap_or("").to_ascii_lowercase();
            codes.iter().any(|c| l == *c || l.starts_with(&format!("{c}-")))
        };
        in_group()
            .find(wanted)
            .or_else(|| in_group().find(|r| r.default))
            .or_else(|| in_group().next())
            .map(|r| r.uri.as_str())
    }
}

/// `(offset, length)` of an `EXT-X-BYTERANGE`.
pub type ByteRange = (u64, u64);

pub struct Segment {
    pub start: f64,
    pub duration: f64,
    pub uri: String,
    pub range: Option<ByteRange>,
    pub key: Option<String>,
    pub map: Option<(String, Option<ByteRange>)>,
}

pub fn is_playlist(body: &str) -> bool {
    body.trim_start_matches('\u{feff}').trim_start().starts_with("#EXTM3U")
}

/// Parses an attribute list (`A=1,B="x,y"`).
fn attrs(s: &str) -> HashMap<String, String> {
    let mut out = HashMap::new();
    let mut rest = s;
    while let Some(eq) = rest.find('=') {
        let key = rest[..eq].trim().trim_start_matches(',').trim().to_string();
        let after = &rest[eq + 1..];
        let (value, next) = if let Some(q) = after.strip_prefix('"') {
            match q.find('"') {
                Some(end) => (&q[..end], &q[end + 1..]),
                None => (q, ""),
            }
        } else {
            match after.find(',') {
                Some(end) => (&after[..end], &after[end..]),
                None => (after, ""),
            }
        };
        out.insert(key, value.trim().to_string());
        rest = next;
    }
    out
}

fn non_empty(v: Option<&String>) -> Option<String> {
    v.filter(|s| !s.is_empty()).cloned()
}

/// `None` when the playlist has no `EXT-X-STREAM-INF` (it is a media playlist then).
pub fn parse_master(body: &str) -> Option<Master> {
    let mut variants = Vec::new();
    let (mut audio, mut subtitles): (Vec<Track>, Vec<Track>) = (Vec::new(), Vec::new());
    let mut renditions = Vec::new();
    let mut pending: Option<(Variant, Option<String>)> = None;
    for line in body.lines().map(str::trim) {
        if let Some(a) = line.strip_prefix("#EXT-X-STREAM-INF:") {
            let a = attrs(a);
            let (width, height) = a
                .get("RESOLUTION")
                .and_then(|r| r.split_once(['x', 'X']))
                .map(|(w, h)| (w.parse().ok(), h.parse().ok()))
                .unwrap_or((None, None));
            let info = Variant {
                width,
                height,
                bandwidth: a.get("BANDWIDTH").and_then(|b| b.parse().ok()),
                codecs: non_empty(a.get("CODECS")),
            };
            pending = Some((info, non_empty(a.get("AUDIO"))));
        } else if let Some(a) = line.strip_prefix("#EXT-X-MEDIA:") {
            let a = attrs(a);
            let track = Track { lang: non_empty(a.get("LANGUAGE")), name: non_empty(a.get("NAME")) };
            let list = match a.get("TYPE").map(String::as_str) {
                Some("AUDIO") => {
                    if let (Some(group), Some(uri)) = (non_empty(a.get("GROUP-ID")), non_empty(a.get("URI"))) {
                        let default = a.get("DEFAULT").is_some_and(|d| d == "YES");
                        renditions.push(AudioRendition { group, lang: track.lang.clone(), default, uri });
                    }
                    &mut audio
                }
                Some("SUBTITLES") => &mut subtitles,
                _ => continue,
            };
            if !list.contains(&track) {
                list.push(track);
            }
        } else if !line.is_empty()
            && !line.starts_with('#')
            && let Some((info, audio_group)) = pending.take()
        {
            variants.push(MasterVariant { info, uri: line.to_string(), audio_group });
        }
    }
    if variants.is_empty() {
        return None;
    }
    // Stable sort: pixels, then bitrate. `prefetch?variant=<i>` indexes this order.
    variants.sort_by_key(|v| {
        let v = &v.info;
        let px = u64::from(v.width.unwrap_or(0)) * u64::from(v.height.unwrap_or(0));
        std::cmp::Reverse((px.max(u64::from(v.height.unwrap_or(0))), v.bandwidth.unwrap_or(0)))
    });
    Some(Master { variants, audio, subtitles, renditions })
}

fn byte_range(spec: &str, prev_end: u64) -> Option<ByteRange> {
    let (len, off) = match spec.split_once('@') {
        Some((l, o)) => (l.trim().parse().ok()?, o.trim().parse().ok()?),
        None => (spec.trim().parse().ok()?, prev_end),
    };
    Some((off, len))
}

pub fn parse_media(body: &str) -> Vec<Segment> {
    let mut out = Vec::new();
    let (mut t, mut dur) = (0.0f64, None::<f64>);
    let mut range = None::<ByteRange>;
    let mut prev_end = 0u64;
    let mut key = None::<String>;
    let mut map = None::<(String, Option<ByteRange>)>;
    for line in body.lines().map(str::trim) {
        if let Some(d) = line.strip_prefix("#EXTINF:") {
            dur = d.split(',').next().and_then(|d| d.trim().parse().ok());
        } else if let Some(r) = line.strip_prefix("#EXT-X-BYTERANGE:") {
            range = byte_range(r, prev_end);
        } else if let Some(a) = line.strip_prefix("#EXT-X-KEY:") {
            key = non_empty(attrs(a).get("URI")).filter(|u| !u.starts_with("data:"));
        } else if let Some(a) = line.strip_prefix("#EXT-X-MAP:") {
            let a = attrs(a);
            map = non_empty(a.get("URI")).map(|u| (u, a.get("BYTERANGE").and_then(|r| byte_range(r, 0))));
        } else if !line.is_empty() && !line.starts_with('#') {
            let duration = dur.take().unwrap_or(0.0);
            if let Some((off, len)) = range {
                prev_end = off + len;
            }
            out.push(Segment {
                start: t,
                duration,
                uri: line.to_string(),
                range: range.take(),
                key: key.clone(),
                map: map.clone(),
            });
            t += duration;
        }
    }
    out
}

/// Local URL that proxies `target` (resolved against `base`) for stream `id`.
pub fn wrap(id: &str, base: &Url, target: &str) -> String {
    let abs = base.join(target).map(String::from).unwrap_or_else(|_| target.to_string());
    let enc: String = url::form_urlencoded::byte_serialize(abs.as_bytes()).collect();
    // The signature is what `/r` accepts: it only fetches addresses that came out of this function.
    format!("/api/stream/{id}/r?u={enc}&s={}", crate::guard::sign_stream(id, &abs))
}

/// Rewrites every URI of a master or media playlist (variant / segment lines and all
/// `URI="..."` attributes: keys, maps, audio and subtitle renditions) to go through the proxy.
pub fn rewrite(id: &str, base: &Url, body: &str) -> String {
    let mut out = String::with_capacity(body.len() * 2);
    for line in body.lines() {
        let t = line.trim();
        if t.starts_with('#') {
            let mut rest = t;
            while let Some(i) = rest.find("URI=\"") {
                let after = &rest[i + 5..];
                let Some(end) = after.find('"') else { break };
                out.push_str(&rest[..i + 5]);
                let uri = &after[..end];
                if uri.starts_with("data:") {
                    out.push_str(uri);
                } else {
                    out.push_str(&wrap(id, base, uri));
                }
                rest = &after[end..];
            }
            out.push_str(rest);
        } else if !t.is_empty() {
            out.push_str(&wrap(id, base, t));
        }
        out.push('\n');
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    const MASTER: &str = "#EXTM3U\n\
        #EXT-X-MEDIA:TYPE=AUDIO,GROUP-ID=\"a\",NAME=\"English, 5.1\",LANGUAGE=\"eng\",URI=\"a/en.m3u8\"\n\
        #EXT-X-MEDIA:TYPE=SUBTITLES,GROUP-ID=\"s\",NAME=\"Deutsch\",LANGUAGE=\"ger\",URI=\"s/de.m3u8\"\n\
        #EXT-X-STREAM-INF:BANDWIDTH=800000,RESOLUTION=640x360,CODECS=\"avc1.4d401e,mp4a.40.2\"\n360.m3u8\n\
        #EXT-X-STREAM-INF:BANDWIDTH=6800000,RESOLUTION=1920x1080\n/hd/1080.m3u8?t=1&x=2\n\
        #EXT-X-STREAM-INF:BANDWIDTH=5000000,RESOLUTION=1920x1080\n1080b.m3u8\n";

    #[test]
    fn master_sorted_best_first() {
        let m = parse_master(MASTER).unwrap();
        let v: Vec<_> = m.variants.iter().map(|v| (v.info.height, v.info.bandwidth, v.uri.as_str())).collect();
        assert_eq!(
            v,
            [
                (Some(1080), Some(6_800_000), "/hd/1080.m3u8?t=1&x=2"),
                (Some(1080), Some(5_000_000), "1080b.m3u8"),
                (Some(360), Some(800_000), "360.m3u8")
            ]
        );
        assert_eq!(m.variants[2].info.codecs.as_deref(), Some("avc1.4d401e,mp4a.40.2"));
        assert_eq!(m.audio_uri(&m.variants[0], "any"), None); // no AUDIO group on that variant
        assert_eq!(m.audio, [Track { lang: Some("eng".into()), name: Some("English, 5.1".into()) }]);
        assert_eq!(m.subtitles[0].lang.as_deref(), Some("ger"));
        assert!(parse_master("#EXTM3U\n#EXTINF:4,\na.ts\n").is_none());
    }

    #[test]
    fn separate_audio_by_language() {
        let m = parse_master(
            "#EXTM3U\n\
             #EXT-X-MEDIA:TYPE=AUDIO,GROUP-ID=\"a\",NAME=\"Italiano\",LANGUAGE=\"ita\",DEFAULT=YES,URI=\"it.m3u8\"\n\
             #EXT-X-MEDIA:TYPE=AUDIO,GROUP-ID=\"a\",NAME=\"English\",LANGUAGE=\"eng\",URI=\"en.m3u8\"\n\
             #EXT-X-STREAM-INF:BANDWIDTH=1,RESOLUTION=1280x720,AUDIO=\"a\"\nv.m3u8\n",
        )
        .unwrap();
        let v = &m.variants[0];
        assert_eq!(m.audio_uri(v, "en"), Some("en.m3u8"));
        assert_eq!(m.audio_uri(v, "de"), Some("it.m3u8")); // not offered: the default
        assert_eq!(m.audio_uri(v, "any"), Some("it.m3u8"));
    }

    #[test]
    fn media_segments_keys_ranges() {
        let body = "#EXTM3U\n#EXT-X-MAP:URI=\"init.mp4\",BYTERANGE=\"700@0\"\n\
            #EXT-X-KEY:METHOD=AES-128,URI=\"k.key\"\n\
            #EXTINF:4.0,\n#EXT-X-BYTERANGE:1000@700\nmain.mp4\n\
            #EXTINF:6.5,\n#EXT-X-BYTERANGE:500\nmain.mp4\n#EXT-X-ENDLIST\n";
        let s = parse_media(body);
        assert_eq!(s.len(), 2);
        assert_eq!((s[0].start, s[0].duration, s[0].range), (0.0, 4.0, Some((700, 1000))));
        assert_eq!((s[1].start, s[1].duration, s[1].range), (4.0, 6.5, Some((1700, 500))));
        assert_eq!(s[1].key.as_deref(), Some("k.key"));
        assert_eq!(s[0].map, Some(("init.mp4".to_string(), Some((0, 700)))));
    }

    #[test]
    fn rewrite_all_uris() {
        let base = Url::parse("https://cdn.example/x/y/master.m3u8?tok=1").unwrap();
        // Every rewritten address carries the signature of (stream, target).
        let local = |target: &str| {
            let enc: String = url::form_urlencoded::byte_serialize(target.as_bytes()).collect();
            format!("/api/stream/ab12/r?u={enc}&s={}", crate::guard::sign_stream("ab12", target))
        };
        let out = rewrite("ab12", &base, MASTER);
        assert!(out.contains(&format!("URI=\"{}\"", local("https://cdn.example/x/y/a/en.m3u8"))));
        assert!(out.contains("r?u=https%3A%2F%2Fcdn.example%2Fx%2Fy%2Fa%2Fen.m3u8&s="));
        assert!(out.contains(&format!("
{}
", local("https://cdn.example/hd/1080.m3u8?t=1&x=2"))));
        assert!(out.contains("NAME=\"English, 5.1\",LANGUAGE=\"eng\""));
        let key = rewrite("ab12", &base, "#EXT-X-KEY:METHOD=AES-128,URI=\"https://k.example/k\",IV=0x1
seg.ts");
        assert_eq!(
            key,
            format!("#EXT-X-KEY:METHOD=AES-128,URI=\"{}\",IV=0x1
{}
", local("https://k.example/k"), local("https://cdn.example/x/y/seg.ts"))
        );
    }
}
