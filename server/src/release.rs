//! Release-name parsing, merging of equal torrents and ranking by the saved filter.

use std::{collections::HashMap, sync::LazyLock};

use regex::Regex;
use serde::Serialize;

use crate::scan::Filter;

#[derive(Debug, Default, Clone, PartialEq, Serialize)]
pub struct Release {
    /// Normalized title part of the name (lowercase words before the first tag).
    pub title: String,
    pub year: Option<u32>,
    pub season: Option<u32>,
    pub episode: Option<u32>,
    /// A whole season (or series) instead of one episode.
    pub pack: bool,
    /// 480 | 576 | 720 | 1080 | 2160
    pub height: Option<u32>,
    /// "Remux" | "BluRay" | "WEB-DL" | "WEBRip" | "HDTV" | "DVD" | "CAM"
    pub source: Option<&'static str>,
    /// "HEVC" | "H.264" | "AV1" | "XviD"
    pub codec: Option<&'static str>,
    /// "DV" | "HDR10+" | "HDR"
    pub hdr: Option<&'static str>,
    /// "Atmos" | "TrueHD" | "DTS-HD" | "DTS" | "EAC3" | "AC3" | "AAC" | "FLAC" | "Opus"
    pub audio: Option<&'static str>,
    /// ISO 639-1 codes plus "multi"; empty = not stated (scene default: English).
    pub languages: Vec<&'static str>,
}

fn re(pattern: &str) -> Regex {
    Regex::new(pattern).expect("static regex")
}

static SEP: LazyLock<Regex> = LazyLock::new(|| re(r"[^\p{L}\p{N}+]+"));
static EPISODE: LazyLock<Regex> = LazyLock::new(|| re(r"\bs(\d{1,2}) ?e(\d{1,3})\b|\b(\d{1,2})x(\d{2,3})\b"));
static SEASON: LazyLock<Regex> = LazyLock::new(|| re(r"\bs(\d{1,2})\b|\b(?:season|staffel) (\d{1,2})\b"));
static YEAR: LazyLock<Regex> = LazyLock::new(|| re(r"\b(19[2-9]\d|20[0-4]\d)\b"));
static HEIGHT: LazyLock<Regex> = LazyLock::new(|| re(r"\b(2160|1080|720|576|480)[pi]\b|\b(4k|uhd)\b"));

/// (pattern over the space-normalized lowercase name, value); first match wins.
type Table = LazyLock<Vec<(Regex, &'static str)>>;

fn table(rows: &[(&str, &'static str)]) -> Vec<(Regex, &'static str)> {
    // Names are space-normalized, so a tag is a whole word between spaces ("hdr10+" has no \b after it).
    rows.iter().map(|(p, v)| (re(&format!(r"(?:^| )(?:{p})(?: |$)")), *v)).collect()
}

static SOURCE: Table = LazyLock::new(|| {
    table(&[
        ("remux|bdremux", "Remux"),
        ("blu ?ray|bdrip|brrip|bd25|bd50|uhd bd", "BluRay"),
        ("web ?rip", "WEBRip"),
        ("web ?dl|web|amzn|nf|dsnp", "WEB-DL"),
        ("hdtv|pdtv|dsr", "HDTV"),
        ("dvdrip|dvd|dvd[59r]", "DVD"),
        ("cam|camrip|hdcam|telesync|hdts|telecine|hdtc", "CAM"),
    ])
});
static CODEC: Table = LazyLock::new(|| {
    table(&[("x265|h ?265|hevc", "HEVC"), ("x264|h ?264|avc", "H.264"), ("av1", "AV1"), ("xvid|divx", "XviD")])
});
static HDR: Table =
    LazyLock::new(|| table(&[("dolby vision|dovi|dv", "DV"), (r"hdr10\+|hdr10plus", "HDR10+"), ("hdr10|hdr", "HDR")]));
static AUDIO: Table = LazyLock::new(|| {
    table(&[
        ("atmos", "Atmos"),
        ("truehd", "TrueHD"),
        ("dts ?hd|dts ?ma|dts ?x", "DTS-HD"),
        ("dts", "DTS"),
        (r"e ?ac ?3|ddp\d?|dd\+\d?|ddplus", "EAC3"),
        (r"ac ?3|dd\d?|dolby digital", "AC3"),
        (r"aac\d?", "AAC"),
        ("flac", "FLAC"),
        ("opus", "Opus"),
    ])
});
static LANG: Table = LazyLock::new(|| {
    table(&[
        ("german|deutsch|ger|deu", "de"),
        ("multi", "multi"),
        ("english|eng", "en"),
        ("french|truefrench|vostfr|vff", "fr"),
        ("italian|ita", "it"),
        ("spanish|castellano|latino|esp", "es"),
        ("russian|rus", "ru"),
        ("hindi", "hi"),
        ("polish|pl", "pl"),
        ("dutch|nl", "nl"),
    ])
});
/// German scene tag "DL" (dual language: German + English); "WEB-DL" is removed before the check.
static DL: LazyLock<Regex> = LazyLock::new(|| re(r"\b(?:dl|dual)\b"));
static WEB_DL: LazyLock<Regex> = LazyLock::new(|| re(r"\bweb ?dl\b"));

fn first(t: &Table, s: &str) -> Option<&'static str> {
    t.iter().find(|(r, _)| r.is_match(s)).map(|(_, v)| *v)
}

/// Lowercase words separated by single spaces.
pub fn normalize(s: &str) -> String {
    SEP.replace_all(&s.to_lowercase(), " ").trim().to_owned()
}

pub fn parse(name: &str) -> Release {
    let s = normalize(name);
    let mut r = Release::default();
    // Everything before the first tag is the title.
    let mut cut = s.len();

    if let Some(c) = EPISODE.captures(&s) {
        let n = |a: usize, b: usize| c.get(a).or(c.get(b)).and_then(|m| m.as_str().parse().ok());
        (r.season, r.episode) = (n(1, 3), n(2, 4));
        cut = cut.min(c.get(0).map_or(cut, |m| m.start()));
    } else if let Some(c) = SEASON.captures(&s) {
        r.season = c.get(1).or(c.get(2)).and_then(|m| m.as_str().parse().ok());
        r.pack = true;
        cut = cut.min(c.get(0).map_or(cut, |m| m.start()));
    }
    if let Some(c) = HEIGHT.captures(&s) {
        r.height = c.get(1).and_then(|m| m.as_str().parse().ok()).or(Some(2160));
        cut = cut.min(c.get(0).map_or(cut, |m| m.start()));
    }
    // The last year is the release year ("2001 A Space Odyssey 1968"); a leading one belongs to the title.
    if let Some(m) = YEAR.find_iter(&s).filter(|m| m.start() > 0).last() {
        r.year = m.as_str().parse().ok();
        cut = cut.min(m.start());
    }
    if cut == s.len() {
        // No year/episode/resolution: the first quality tag ends the title.
        for t in [&SOURCE, &CODEC, &HDR, &AUDIO] {
            for m in t.iter().filter_map(|(re, _)| re.find(&s)) {
                cut = cut.min(m.start());
            }
        }
    }
    r.title = s[..cut].trim().to_owned();
    // Tags are only looked for after the title, so "Web of Lies 2023" or "Cam 2018" keep their titles.
    let tags = &s[cut..];
    r.source = first(&SOURCE, tags);
    r.codec = first(&CODEC, tags);
    r.hdr = first(&HDR, tags);
    r.audio = first(&AUDIO, tags);
    r.languages = LANG.iter().filter(|(re, _)| re.is_match(tags)).map(|(_, v)| *v).collect();
    if DL.is_match(&WEB_DL.replace_all(tags, " ")) && r.languages.contains(&"de") && !r.languages.contains(&"en") {
        r.languages.push("en");
    }
    if s.contains("complete") || s.contains("komplett") {
        r.pack = r.episode.is_none();
    }
    r
}

/// One torrent as reported by one source.
#[derive(Debug, Clone, Default)]
pub struct Hit {
    pub source: &'static str,
    pub infohash: String,
    pub name: String,
    pub size: u64,
    pub seeders: u32,
    pub leechers: u32,
    pub trackers: Vec<String>,
    /// File inside the torrent that is the requested movie/episode (IMDb sources know it).
    pub file_index: Option<usize>,
    /// Found by IMDb id: certainly the requested title.
    pub exact: bool,
}

#[derive(Debug, Serialize)]
pub struct Torrent {
    pub infohash: String,
    pub name: String,
    pub size: u64,
    /// Sum over all sources that list this torrent.
    pub seeders: u32,
    /// Highest value a single source reported.
    pub seeders_max: u32,
    pub leechers: u32,
    pub sources: Vec<&'static str>,
    pub trackers: Vec<String>,
    pub file_index: Option<usize>,
    pub exact: bool,
    pub release: Release,
    /// The saved filter's minimum height is not met (or unknown).
    pub below_min: bool,
    pub score: i64,
    pub magnet: String,
}

/// 40 hex chars, lowercase; also accepts the 32-char base32 form.
pub fn clean_hash(s: &str) -> Option<String> {
    let s = s.trim();
    if s.len() == 40 && s.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Some(s.to_ascii_lowercase());
    }
    if s.len() != 32 {
        return None;
    }
    let mut bits = 0u64;
    let mut n = 0;
    let mut out = String::with_capacity(40);
    for c in s.bytes() {
        let v = match c.to_ascii_uppercase() {
            c @ b'A'..=b'Z' => c - b'A',
            c @ b'2'..=b'7' => c - b'2' + 26,
            _ => return None,
        };
        bits = (bits << 5) | u64::from(v);
        n += 5;
        while n >= 4 {
            n -= 4;
            out.push(char::from_digit(((bits >> n) & 0xf) as u32, 16)?);
        }
    }
    Some(out)
}

pub fn magnet(hash: &str, name: &str, trackers: &[String]) -> String {
    let enc = |s: &str| url::form_urlencoded::byte_serialize(s.as_bytes()).collect::<String>();
    let mut m = format!("magnet:?xt=urn:btih:{hash}&dn={}", enc(name));
    for t in trackers {
        m.push_str("&tr=");
        m.push_str(&enc(t));
    }
    m
}

/// Hash, display name and trackers of a magnet link.
pub fn parse_magnet(link: &str) -> Option<(String, Option<String>, Vec<String>)> {
    let url = url::Url::parse(link).ok().filter(|u| u.scheme() == "magnet")?;
    let (mut hash, mut name, mut trackers) = (None, None, Vec::new());
    for (k, v) in url.query_pairs() {
        match &*k {
            "xt" => hash = hash.or(v.strip_prefix("urn:btih:").and_then(clean_hash)),
            "dn" => name = Some(v.into_owned()),
            "tr" => trackers.push(v.into_owned()),
            _ => {}
        }
    }
    Some((hash?, name, trackers))
}

fn score(t: &Torrent, f: &Filter) -> i64 {
    let r = &t.release;
    let has = |l: &str| r.languages.contains(&l);
    let lang = match f.audio_lang.as_str() {
        "de" if has("de") => 1000,
        "de" if has("multi") => 400,
        // Untagged releases are English by convention.
        "en" if has("en") || r.languages.is_empty() => 600,
        "en" if has("multi") => 400,
        _ => 0,
    };
    let source = match r.source {
        Some("Remux") => 30,
        Some("BluRay") => 25,
        Some("WEB-DL") => 20,
        Some("WEBRip") => 15,
        Some("HDTV") => 8,
        Some("DVD") => 3,
        Some("CAM") => -400,
        _ => 0,
    };
    let height = i64::from(r.height.unwrap_or(0));
    let swarm = ((1.0 + f64::from(t.seeders)).log2() * 25.0).min(300.0) as i64;
    let megabytes = (t.size as f64 / 1e6).max(1.0);
    let sort = match f.sort.as_str() {
        "bitrate" => (megabytes.log2() * 20.0) as i64 + height / 20,
        "speed" => swarm * 2 + height / 20,
        _ => height / 5 + source + i64::from(r.hdr.is_some()) * 5,
    };
    lang + sort + swarm + source.min(0)
        - if t.seeders == 0 { 2000 } else { 0 }
        - if t.below_min { 1200 } else { 0 }
        + if t.exact { 40 } else { 0 }
}

/// Merges hits with the same infohash (seeders summed, trackers united) and sorts best first.
pub fn merge_and_rank(hits: Vec<Hit>, filter: &Filter) -> Vec<Torrent> {
    let mut map: HashMap<String, Torrent> = HashMap::new();
    let mut order = Vec::new();
    for h in hits {
        let t = map.entry(h.infohash.clone()).or_insert_with(|| {
            order.push(h.infohash.clone());
            Torrent {
                infohash: h.infohash.clone(),
                name: String::new(),
                size: 0,
                seeders: 0,
                seeders_max: 0,
                leechers: 0,
                sources: Vec::new(),
                trackers: Vec::new(),
                file_index: None,
                exact: false,
                release: Release::default(),
                below_min: false,
                score: 0,
                magnet: String::new(),
            }
        });
        if t.sources.contains(&h.source) {
            continue;
        }
        t.sources.push(h.source);
        t.seeders += h.seeders;
        t.seeders_max = t.seeders_max.max(h.seeders);
        t.leechers = t.leechers.max(h.leechers);
        t.size = t.size.max(h.size);
        t.exact |= h.exact;
        t.file_index = t.file_index.or(h.file_index);
        // The longest name carries the most tags.
        if h.name.len() > t.name.len() {
            t.name = h.name;
        }
        for tr in h.trackers {
            if !t.trackers.contains(&tr) {
                t.trackers.push(tr);
            }
        }
    }
    let mut out: Vec<Torrent> = order.into_iter().filter_map(|k| map.remove(&k)).collect();
    for t in &mut out {
        t.release = parse(&t.name);
        t.below_min = filter.min_height > 0 && t.release.height.is_none_or(|h| h < filter.min_height);
        t.magnet = magnet(&t.infohash, &t.name, &t.trackers);
        t.score = score(t, filter);
    }
    out.sort_by(|a, b| b.score.cmp(&a.score).then(b.seeders.cmp(&a.seeders)));
    out
}

/// "1.4 GB", "700 MiB", "1,2 GiB" → bytes.
pub fn parse_size(text: &str) -> u64 {
    static SIZE: LazyLock<Regex> = LazyLock::new(|| re(r"(?i)([\d.,]+)\s*([kmgt])i?b"));
    let Some(c) = SIZE.captures(text) else { return 0 };
    let n: f64 = c[1].replace(',', ".").parse().unwrap_or(0.0);
    let exp = match c[2].to_ascii_lowercase().as_str() {
        "k" => 1,
        "m" => 2,
        "g" => 3,
        _ => 4,
    };
    (n * 1024f64.powi(exp)) as u64
}

#[cfg(test)]
mod tests {
    use super::*;

    fn filter(min_height: u32, lang: &str, sort: &str) -> Filter {
        Filter { min_height, audio_lang: lang.into(), sort: sort.into() }
    }

    #[test]
    fn parses_a_german_dual_language_release() {
        let r = parse("Fight.Club.1999.German.DL.2160p.UHD.BluRay.HDR.HEVC.Remux-NIMA4K");
        assert_eq!(r.title, "fight club");
        assert_eq!((r.year, r.height, r.source, r.codec, r.hdr), (Some(1999), Some(2160), Some("Remux"), Some("HEVC"), Some("HDR")));
        assert_eq!(r.languages, vec!["de", "en"]);
        assert!(!r.pack && r.episode.is_none());
    }

    #[test]
    fn parses_episode_pack_and_multi() {
        let e = parse("The.Expanse.S03E07.MULTi.1080p.WEB-DL.DD5.1.H.264-GRP");
        assert_eq!((e.title.as_str(), e.season, e.episode, e.pack), ("the expanse", Some(3), Some(7), false));
        assert_eq!((e.source, e.codec, e.audio, e.languages.as_slice()), (Some("WEB-DL"), Some("H.264"), Some("AC3"), &["multi"][..]));

        let p = parse("Dark.S02.GERMAN.1080p.NF.WEBRip.DDP5.1.x264-TvR");
        assert_eq!((p.season, p.episode, p.pack, p.source, p.audio), (Some(2), None, true, Some("WEBRip"), Some("EAC3")));
        assert_eq!(p.languages, vec!["de"]);

        let st = parse("Dark Staffel 1 (2017) 720p");
        assert_eq!((st.title.as_str(), st.season, st.pack, st.year, st.height), ("dark", Some(1), true, Some(2017), Some(720)));

        let x = parse("Some Show 2x05 HDTV XviD");
        assert_eq!((x.season, x.episode, x.source, x.codec), (Some(2), Some(5), Some("HDTV"), Some("XviD")));
    }

    #[test]
    fn web_dl_alone_is_not_a_language_tag_and_titles_are_not_tags() {
        let r = parse("Sintel (2010) 1080p WEB-DL DTS-HD MA 5.1 x265");
        assert!(r.languages.is_empty());
        assert_eq!((r.title.as_str(), r.audio, r.source), ("sintel", Some("DTS-HD"), Some("WEB-DL")));
        let t = parse("Web.of.Lies.2023.720p.HDTV.x264");
        assert_eq!((t.title.as_str(), t.source), ("web of lies", Some("HDTV")));
        let y = parse("2001.A.Space.Odyssey.1968.2160p.DV.HDR10+.TrueHD.Atmos");
        assert_eq!((y.title.as_str(), y.year, y.hdr, y.audio), ("2001 a space odyssey", Some(1968), Some("DV"), Some("Atmos")));
    }

    #[test]
    fn sizes_and_hashes() {
        assert_eq!(parse_size("1.5 GiB"), 1_610_612_736);
        assert_eq!(parse_size("749,18 MB"), (749.18 * 1024.0 * 1024.0) as u64);
        assert_eq!(clean_hash("08ADA5A7A6183AAE1E09D831DF6748D566095A10").unwrap(), "08ada5a7a6183aae1e09d831df6748d566095a10");
        assert_eq!(clean_hash("BCW2LJ5GDA5K4HQJ3AY56Z2I2VTASWQQ").unwrap(), "08ada5a7a6183aae1e09d831df6748d566095a10");
        assert!(clean_hash("xyz").is_none());
        let (h, n, tr) = parse_magnet("magnet:?xt=urn:btih:08ADA5A7A6183AAE1E09D831DF6748D566095A10&dn=Sintel+2010&tr=udp%3A%2F%2Fa.b%3A1").unwrap();
        assert_eq!((h.as_str(), n.as_deref(), tr.as_slice()), ("08ada5a7a6183aae1e09d831df6748d566095a10", Some("Sintel 2010"), &["udp://a.b:1".to_owned()][..]));
    }

    fn hit(source: &'static str, hash: &str, name: &str, seeders: u32, trackers: &[&str]) -> Hit {
        Hit {
            source,
            infohash: hash.into(),
            name: name.into(),
            size: 2_000_000_000,
            seeders,
            trackers: trackers.iter().map(|t| t.to_string()).collect(),
            ..Hit::default()
        }
    }

    #[test]
    fn merges_equal_hashes_and_sums_seeders() {
        let out = merge_and_rank(
            vec![
                hit("knaben", "aa", "Movie 2020 1080p", 10, &["udp://a"]),
                hit("tpb", "aa", "Movie.2020.1080p.BluRay.x264-GRP", 5, &["udp://a", "udp://b"]),
                hit("tpb", "aa", "Movie.2020.1080p.BluRay.x264-GRP", 5, &[]),
                hit("yts", "bb", "Movie 2020 720p", 3, &[]),
            ],
            &filter(0, "any", "quality"),
        );
        assert_eq!(out.len(), 2);
        let a = &out[0];
        assert_eq!((a.infohash.as_str(), a.seeders, a.seeders_max), ("aa", 15, 10));
        assert_eq!(a.sources, vec!["knaben", "tpb"]);
        assert_eq!(a.trackers, vec!["udp://a", "udp://b"]);
        assert_eq!(a.name, "Movie.2020.1080p.BluRay.x264-GRP");
        assert!(a.magnet.starts_with("magnet:?xt=urn:btih:aa&dn=Movie.2020") && a.magnet.contains("&tr=udp%3A%2F%2Fb"));
    }

    #[test]
    fn ranks_by_language_min_height_and_seeders() {
        let hits = || {
            vec![
                hit("a", "en4k", "Film.2020.2160p.WEB-DL.x265", 500, &[]),
                hit("a", "de1080", "Film.2020.German.DL.1080p.BluRay.x264", 20, &[]),
                hit("a", "de720", "Film.2020.German.720p.WEB.x264", 80, &[]),
                hit("a", "dead", "Film.2020.German.DL.2160p.BluRay.x265", 0, &[]),
                hit("a", "cam", "Film.2020.German.HDCAM.x264", 900, &[]),
            ]
        };
        let ids = |f: Filter| merge_and_rank(hits(), &f).into_iter().map(|t| t.infohash).collect::<Vec<_>>();
        // German wanted: live German releases first (better quality first), CAM and dead torrents last.
        assert_eq!(ids(filter(0, "de", "quality")), ["de1080", "de720", "en4k", "cam", "dead"]);
        // Minimum 1080p beats the language wish: the 720p release falls behind the English 4K one.
        let min = ids(filter(1080, "de", "quality"));
        assert!(min[0] == "de1080" && min.iter().position(|i| i == "de720") > min.iter().position(|i| i == "en4k"));
        // No language wish: resolution decides, then seeders.
        assert_eq!(ids(filter(0, "any", "quality"))[0], "en4k");
        assert_eq!(ids(filter(0, "any", "speed"))[0], "en4k");
        assert_eq!(ids(filter(0, "en", "quality"))[0], "en4k");
    }
}
