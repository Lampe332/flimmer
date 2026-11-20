//! Release-name parsing, merging of equal torrents, filters with facets and ranking.

use std::{
    collections::{BTreeMap, HashMap},
    sync::LazyLock,
};

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
    /// 480 | 576 | 720 | 1080 | 2160 | 4320
    pub height: Option<u32>,
    /// "Remux" | "BluRay" | "WEB-DL" | "WEBRip" | "HDTV" | "DVD" | "SCR" | "CAM" | "TS" | "TC"
    pub source: Option<&'static str>,
    /// Filmed in a cinema or a pre-retail copy (CAM, TS, TC, screener).
    pub cam: bool,
    /// "HEVC" | "H.264" | "AV1" | "XviD" | "VC-1" | "MPEG-2"
    pub codec: Option<&'static str>,
    /// 8 | 10 | 12
    pub bit_depth: Option<u8>,
    /// The best of `hdr_all`.
    pub hdr: Option<&'static str>,
    /// "DV" | "HDR10+" | "HDR10" | "HDR" | "HLG", best first.
    pub hdr_all: Vec<&'static str>,
    /// "Atmos" | "TrueHD" | "DTS-HD MA" | "DTS-HD" | "DTS" | "EAC3" | "AC3" | "AAC" | "FLAC" | "Opus" | "MP3" | "PCM"
    pub audio: Option<&'static str>,
    /// "7.1" | "5.1" | "2.0" | "1.0"
    pub channels: Option<&'static str>,
    /// Audio languages: ISO 639-1 codes plus "multi" / "nordic"; empty = not stated (scene default: English).
    pub languages: Vec<&'static str>,
    /// Subtitle languages named in the release ("German Subbed", "VOSTFR", "ESubs" …).
    pub subtitles: Vec<&'static str>,
    /// Two audio tracks ("German DL" = German + original, "Dual Audio").
    pub dual: bool,
    /// German audio taken from elsewhere: "mic" (MD) | "line" (LD) | "dubbed" (AC3D, Dubbed).
    pub dub: Option<&'static str>,
    pub group: Option<String>,
    pub proper: bool,
    pub repack: bool,
    /// "Extended" | "Uncut" | "Director's Cut" | "Unrated" | "IMAX" | "Remastered" | "Theatrical"
    pub edition: Option<&'static str>,
    #[serde(rename = "3d")]
    pub three_d: bool,
}

fn re(pattern: &str) -> Regex {
    Regex::new(pattern).expect("static regex")
}

static SEP: LazyLock<Regex> = LazyLock::new(|| re(r"[^\p{L}\p{N}+]+"));
static EPISODE: LazyLock<Regex> = LazyLock::new(|| re(r"\bs(\d{1,2}) ?e(\d{1,3})\b|\b(\d{1,2})x(\d{2,3})\b"));
static SEASON: LazyLock<Regex> = LazyLock::new(|| re(r"\bs(\d{1,2})\b|\b(?:season|staffel) (\d{1,2})\b"));
static YEAR: LazyLock<Regex> = LazyLock::new(|| re(r"\b(19[2-9]\d|20[0-4]\d)\b"));
static HEIGHT: LazyLock<Regex> =
    LazyLock::new(|| re(r"\b(4320|2160|1080|720|576|480)[pi]\b|\b(8k)\b|\b(4k|uhd)\b|\b(fullhd|fhd)\b|\b\d{3,4}x(\d{3,4})\b"));
static EXT: LazyLock<Regex> = LazyLock::new(|| re(r"(?i)\.(mkv|mp4|m4v|avi|mov|webm|ts|m2ts|wmv|mpg|mpeg|srt|ass|sub|idx|nfo)$"));
/// "-GROUP" at the end, optionally followed by a bracket tag ("-GRP[rarbg]"), or a closing "[GROUP]".
static GROUP: LazyLock<Regex> = LazyLock::new(|| re(r"-([A-Za-z0-9][A-Za-z0-9_.]{1,19})(?:\s*[\[(][^\])]*[\])])*$|[\[(]([A-Za-z0-9][A-Za-z0-9_.-]{1,19})[\])]$"));

/// (pattern over the space-normalized lowercase name, value); first match wins.
type Table = LazyLock<Vec<(Regex, &'static str)>>;

fn table(rows: &[(&str, &'static str)]) -> Vec<(Regex, &'static str)> {
    // Names are space-normalized, so a tag is a whole word between spaces ("hdr10+" has no \b after it).
    rows.iter().map(|(p, v)| (re(&format!(r"(?:^| )(?:{p})(?: |$)")), *v)).collect()
}

static SOURCE: Table = LazyLock::new(|| {
    table(&[
        ("remux|bdremux", "Remux"),
        ("blu ?ray|bdrip|brrip|bd25|bd50|bdmv|uhd bd", "BluRay"),
        ("web ?rip", "WEBRip"),
        ("web ?dl|web|amzn|nf|dsnp|atvp|hmax", "WEB-DL"),
        ("hdtv|pdtv|dsr|satrip|tvrip", "HDTV"),
        ("dvdscr|bdscr|screener|scr", "SCR"),
        ("dvdrip|dvd|dvd[59r]", "DVD"),
        ("cam|camrip|hdcam|hqcam", "CAM"),
        ("telesync|hdts|hd ts|tsrip|ts", "TS"),
        ("telecine|hdtc|hd tc|tc", "TC"),
    ])
});
static CODEC: Table = LazyLock::new(|| {
    table(&[
        ("x265|h ?265|hevc", "HEVC"),
        ("x264|h ?264|avc", "H.264"),
        ("av1", "AV1"),
        ("xvid|divx", "XviD"),
        ("vc ?1", "VC-1"),
        ("mpeg ?2", "MPEG-2"),
    ])
});
static HDR: Table = LazyLock::new(|| {
    table(&[
        ("dolby vision|dovi|dv", "DV"),
        (r"hdr10\+|hdr10plus|hdr10p", "HDR10+"),
        ("hdr10", "HDR10"),
        ("hdr", "HDR"),
        ("hlg", "HLG"),
    ])
});
static AUDIO: Table = LazyLock::new(|| {
    table(&[
        ("atmos", "Atmos"),
        ("truehd", "TrueHD"),
        ("dts ?hd ?ma|dts ?ma", "DTS-HD MA"),
        ("dts ?hd|dtshd|dts ?x|dts ?es", "DTS-HD"),
        ("dts", "DTS"),
        (r"e ?ac ?3d?|ddp\d?|dd\+\d?|ddplus", "EAC3"),
        (r"ac ?3(?:d|ld|md)?|dd\d?|dolby digital", "AC3"),
        (r"aac\d?", "AAC"),
        ("flac", "FLAC"),
        ("opus", "Opus"),
        ("mp3", "MP3"),
        ("l?pcm", "PCM"),
    ])
});
static EDITION: Table = LazyLock::new(|| {
    table(&[
        ("extended", "Extended"),
        ("uncut", "Uncut"),
        ("directors cut|dc", "Director's Cut"),
        ("unrated", "Unrated"),
        ("imax", "IMAX"),
        ("remastered", "Remastered"),
        ("theatrical", "Theatrical"),
    ])
});
/// Words for a language, as an alternation (shared by the audio and the subtitle patterns).
const LANG_WORDS: &[(&str, &str)] = &[
    ("german|deutsch|ger|deu", "de"),
    ("multi|ml", "multi"),
    ("english|eng", "en"),
    ("french|truefrench|vff|vfq|vfi|vf2|vf", "fr"),
    ("italian|ita", "it"),
    ("spanish|castellano|latino|esp|spa", "es"),
    ("russian|rus", "ru"),
    ("hindi|hin", "hi"),
    ("polish|pl|pol", "pl"),
    ("dutch|nl|nld|flemish", "nl"),
    ("japanese|jap|jpn", "ja"),
    ("korean|kor", "ko"),
    ("chinese|chi|mandarin|cantonese", "zh"),
    ("portuguese|por|pt br|dublado", "pt"),
    ("turkish|tur", "tr"),
    ("swedish|swe", "sv"),
    ("danish|dan", "da"),
    ("norwegian|nor", "no"),
    ("finnish|fin", "fi"),
    ("nordic", "nordic"),
    ("czech|cze", "cs"),
    ("hungarian|hun", "hu"),
    ("ukrainian|ukr", "uk"),
    ("arabic|ara", "ar"),
];
static LANG: Table = LazyLock::new(|| table(LANG_WORDS));
/// "<language> subbed", "<language> subs" …: the language of subtitles, not of the audio.
static SUBBED: LazyLock<Vec<(Regex, &'static str)>> = LazyLock::new(|| {
    LANG_WORDS.iter().map(|(p, v)| (re(&format!(r"(?:^| )(?:{p}) (?:subbed|subs|sub|subtitles?|untertitel)(?: |$)")), *v)).collect()
});
/// One-word subtitle tags.
static SUB_WORD: Table = LazyLock::new(|| {
    table(&[
        ("vostfr|vost|subfrench", "fr"),
        ("gersub|germansub", "de"),
        ("engsubs?|esubs?", "en"),
        ("msubs?|multi ?subs?", "multi"),
        ("korsubs?", "ko"),
    ])
});
/// German scene tag "DL" / "Dual" (two audio tracks); "WEB-DL" is removed before the check.
static DL: LazyLock<Regex> = LazyLock::new(|| re(r"\b(?:dl|dual|dual audio)\b"));
static WEB_DL: LazyLock<Regex> = LazyLock::new(|| re(r"\bweb ?dl\b"));
static CHANNELS: LazyLock<Regex> = LazyLock::new(|| re(r"(?:^| |[a-z+])(7 1|5 1|2 0|1 0)(?: |$)|(?:^| )([268])ch(?: |$)"));
static BITS: LazyLock<Regex> = LazyLock::new(|| re(r"(?:^| )(?:(8|10|12) ?bits?|(hi10p?))(?: |$)"));
static DUB: Table = LazyLock::new(|| table(&[("md|ac3md|mic dubbed", "mic"), ("ld|ac3ld|line dubbed", "line"), ("dubbed|ac3d|eac3d|synced", "dubbed")]));
static PROPER: LazyLock<Regex> = LazyLock::new(|| re(r"(?:^| )proper(?: |$)"));
static REPACK: LazyLock<Regex> = LazyLock::new(|| re(r"(?:^| )(?:repack|rerip)(?: |$)"));
static THREE_D: LazyLock<Regex> = LazyLock::new(|| re(r"(?:^| )(?:3d|hsbs|h sbs|half sbs|hou|h ou)(?: |$)"));
/// Tag words that may stand between the title and the year/resolution ("Title.German.DL.1080p").
const PRE_TAGS: &[&str] = &[
    "dl", "ml", "dual", "multi", "dubbed", "subbed", "proper", "repack", "uncut", "extended", "remastered", "unrated", "internal",
    "complete", "komplett", "ac3", "ac3d", "ac3ld", "ac3md", "dts", "md", "ld", "bluray", "web", "hdtv", "uhd", "imax",
];

fn first(t: &Table, s: &str) -> Option<&'static str> {
    t.iter().find(|(r, _)| r.is_match(s)).map(|(_, v)| *v)
}

fn all(t: &[(Regex, &'static str)], s: &str) -> Vec<&'static str> {
    t.iter().filter(|(r, _)| r.is_match(s)).map(|(_, v)| *v).collect()
}

/// Lowercase words separated by single spaces.
pub fn normalize(s: &str) -> String {
    SEP.replace_all(&s.to_lowercase(), " ").trim().to_owned()
}

fn is_language_word(word: &str) -> bool {
    LANG_WORDS.iter().any(|(p, _)| p.split('|').any(|w| w == word))
}

/// Start of the run of tag words that ends right at `cut` ("… german dl | 1080p").
fn pre_tag_start(s: &str, cut: usize, year_follows: bool) -> usize {
    let mut start = cut;
    let mut count = 0;
    let mut languages = 0;
    for word in s[..cut].split(' ').rev().filter(|w| !w.is_empty()) {
        let lang = is_language_word(word) && word.len() > 2;
        if !lang && !PRE_TAGS.contains(&word) {
            break;
        }
        start -= word.len() + usize::from(start > word.len());
        count += 1;
        languages += usize::from(lang);
    }
    // A single language word right before the year is more likely the title ("The Good German 2006").
    let ambiguous = year_follows && count == 1 && languages == 1;
    if count == 0 || ambiguous || s[..start].trim().is_empty() { cut } else { start.min(cut) }
}

pub fn parse(name: &str) -> Release {
    let raw = EXT.replace(name.trim(), "");
    let mut r = Release::default();
    if let Some(c) = GROUP.captures(&raw) {
        let g = c.get(1).or(c.get(2)).map_or("", |m| m.as_str());
        // "WEB-DL", "DTS-HD", "H-264" end in a tag, not in a group.
        let tag = matches!(g.to_ascii_lowercase().as_str(), "dl" | "hd" | "ma" | "es" | "264" | "265" | "rip" | "ray" | "x264" | "x265" | "hevc" | "remux")
            || g.bytes().all(|b| b.is_ascii_digit());
        if !tag {
            r.group = Some(g.to_owned());
        }
    }
    let mut s = normalize(&raw);
    if let Some(g) = &r.group {
        let g = normalize(g);
        if let Some(rest) = s.strip_suffix(&g) {
            s = rest.trim_end().to_owned();
        }
    }
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
        r.height = if let Some(m) = c.get(1) {
            m.as_str().parse().ok()
        } else if c.get(2).is_some() {
            Some(4320)
        } else if c.get(3).is_some() {
            Some(2160)
        } else if c.get(4).is_some() {
            Some(1080)
        } else {
            c.get(5).and_then(|m| m.as_str().parse::<u32>().ok()).map(|h| match h {
                2000.. => 2160,
                1000.. => 1080,
                700.. => 720,
                560.. => 576,
                _ => 480,
            })
        };
        cut = cut.min(c.get(0).map_or(cut, |m| m.start()));
    }
    // The last year is the release year ("2001 A Space Odyssey 1968"); a leading one belongs to the title.
    let mut year_at = None;
    if let Some(m) = YEAR.find_iter(&s).filter(|m| m.start() > 0).last() {
        r.year = m.as_str().parse().ok();
        year_at = Some(m.start());
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
    cut = pre_tag_start(&s, cut, year_at == Some(cut));
    r.title = s[..cut].trim().to_owned();
    // Tags are only looked for after the title, so "Web of Lies 2023" or "Cam 2018" keep their titles.
    let tags = &s[cut..];
    r.source = first(&SOURCE, tags);
    r.cam = matches!(r.source, Some("CAM" | "TS" | "TC" | "SCR"));
    r.codec = first(&CODEC, tags);
    r.hdr_all = all(&HDR, tags);
    if r.hdr_all.contains(&"HDR10+") || r.hdr_all.contains(&"HDR10") {
        r.hdr_all.retain(|h| *h != "HDR");
    }
    if r.hdr_all.contains(&"HDR10+") {
        // "hdr10+" also matches nothing else, but "HDR10 HDR10+" names list both: keep the better one.
        r.hdr_all.retain(|h| *h != "HDR10");
    }
    r.hdr = r.hdr_all.first().copied();
    r.audio = first(&AUDIO, tags);
    r.channels = CHANNELS.captures(tags).and_then(|c| match c.get(1).or(c.get(2)).map(|m| m.as_str()) {
        Some("7 1" | "8") => Some("7.1"),
        Some("5 1" | "6") => Some("5.1"),
        Some("2 0" | "2") => Some("2.0"),
        Some("1 0") => Some("1.0"),
        _ => None,
    });
    r.bit_depth = BITS.captures(tags).map(|c| c.get(1).and_then(|m| m.as_str().parse().ok()).unwrap_or(10));
    if r.bit_depth.is_none() && r.hdr.is_some() {
        r.bit_depth = Some(10);
    }
    r.dub = first(&DUB, tags);
    r.proper = PROPER.is_match(tags);
    r.repack = REPACK.is_match(tags);
    r.edition = first(&EDITION, tags);
    r.three_d = THREE_D.is_match(tags);

    // Subtitle languages first; what names subtitles is not an audio language.
    let mut audio_tags = tags.to_owned();
    for (re, lang) in SUBBED.iter().chain(SUB_WORD.iter()) {
        if re.is_match(&audio_tags) {
            if !r.subtitles.contains(lang) {
                r.subtitles.push(lang);
            }
            audio_tags = re.replace_all(&audio_tags, " ").into_owned();
        }
    }
    r.languages = all(&LANG, &audio_tags);
    r.dual = DL.is_match(&WEB_DL.replace_all(&audio_tags, " "));
    if r.dual && r.languages.contains(&"de") && !r.languages.contains(&"en") {
        r.languages.push("en");
    }
    if r.dual && r.languages.is_empty() {
        r.languages.push("multi");
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
    /// Upload time (unix seconds), when the source tells it.
    pub added: Option<i64>,
    /// Audio languages the source states besides the name (flags, language lines).
    pub languages: Vec<&'static str>,
    /// The source does not report seeders (hash indexes, RSS feeds): `seeders` is no information.
    pub no_seeds: bool,
}

/// Audio languages named in a free text ("English + Russian", "German"), as release codes.
pub fn languages_in(text: &str) -> Vec<&'static str> {
    all(&LANG, &normalize(text))
}

#[derive(Debug, Clone, Serialize)]
pub struct Torrent {
    pub infohash: String,
    pub name: String,
    pub size: u64,
    /// Sum over all sources that list this torrent.
    pub seeders: u32,
    /// Highest value a single source reported.
    pub seeders_max: u32,
    /// No source that lists this torrent reports seeders: 0 means "not known", not "dead".
    pub seeders_unknown: bool,
    pub leechers: u32,
    pub sources: Vec<&'static str>,
    pub trackers: Vec<String>,
    pub file_index: Option<usize>,
    pub exact: bool,
    /// Upload time (unix seconds), the earliest any source reported.
    pub added: Option<i64>,
    pub age_days: Option<u32>,
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

/// How results are ordered: the saved filter, optionally overridden per request.
pub struct Ranking<'a> {
    pub filter: &'a Filter,
    /// "fast" (many seeders, small file: starts quickly) | "quality" | None = the filter's `sort`.
    pub mode: Option<&'a str>,
}

fn score(t: &Torrent, rank: &Ranking) -> i64 {
    let f = rank.filter;
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
        _ => 0,
    };
    let height = i64::from(r.height.unwrap_or(0));
    let swarm = ((1.0 + f64::from(t.seeders)).log2() * 25.0).min(300.0) as i64;
    let megabytes = (t.size as f64 / 1e6).max(1.0);
    let quality = height / 5 + source + i64::from(r.hdr.is_some()) * 5 + i64::from(r.proper || r.repack) * 3;
    let sort = match rank.mode.unwrap_or(f.sort.as_str()) {
        "bitrate" => (megabytes.log2() * 20.0) as i64 + height / 20,
        "speed" => swarm * 2 + height / 20,
        // Many seeders per gigabyte: the first minutes are there soonest.
        "fast" => swarm * 3 - (megabytes.log2() * 18.0) as i64 + height / 40,
        "quality" if rank.mode.is_some() => quality * 2 + (megabytes.log2() * 6.0) as i64,
        _ => quality,
    };
    // "Best quality": a healthy swarm is enough, more seeders do not outweigh a better source.
    let swarm = if rank.mode == Some("quality") { swarm.min(100) } else { swarm };
    lang + sort + swarm
        - if r.cam { 800 } else { 0 }
        - if matches!(r.dub, Some("mic" | "line")) { 250 } else { 0 }
        - if r.three_d { 150 } else { 0 }
        // Unknown seeders (hash indexes, RSS without numbers) rank below every live swarm, above dead ones.
        - match (t.seeders, t.seeders_unknown) {
            (0, true) => 1500,
            (0, false) => 2000,
            _ => 0,
        }
        - if t.below_min { 1200 } else { 0 }
        + if t.exact { 40 } else { 0 }
}

/// Merges hits with the same infohash (seeders summed, trackers united). Unranked.
pub fn merge(hits: Vec<Hit>, now_s: i64) -> Vec<Torrent> {
    let mut map: HashMap<String, Torrent> = HashMap::new();
    let mut langs: HashMap<String, Vec<&'static str>> = HashMap::new();
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
                seeders_unknown: true,
                leechers: 0,
                sources: Vec::new(),
                trackers: Vec::new(),
                file_index: None,
                exact: false,
                added: None,
                age_days: None,
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
        t.seeders_unknown &= h.no_seeds;
        let known = langs.entry(h.infohash.clone()).or_default();
        for l in &h.languages {
            if !known.contains(l) {
                known.push(l);
            }
        }
        t.seeders += h.seeders;
        t.seeders_max = t.seeders_max.max(h.seeders);
        t.leechers = t.leechers.max(h.leechers);
        t.size = t.size.max(h.size);
        t.exact |= h.exact;
        t.file_index = t.file_index.or(h.file_index);
        t.added = match (t.added, h.added.filter(|a| *a > 0)) {
            (Some(a), Some(b)) => Some(a.min(b)),
            (a, b) => a.or(b),
        };
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
        // Languages a source states besides the name (Comet flags, MediaFusion and Zilean lists).
        for l in langs.remove(&t.infohash).unwrap_or_default() {
            if !t.release.languages.contains(&l) {
                t.release.languages.push(l);
            }
        }
        t.age_days = t.added.map(|a| ((now_s - a).max(0) / 86_400) as u32);
        t.magnet = magnet(&t.infohash, &t.name, &t.trackers);
    }
    out
}

/// Scores and sorts best first.
pub fn rank(list: &mut [Torrent], rank: &Ranking) {
    for t in list.iter_mut() {
        t.below_min = rank.filter.min_height > 0 && t.release.height.is_none_or(|h| h < rank.filter.min_height);
        t.score = score(t, rank);
    }
    list.sort_by(|a, b| b.score.cmp(&a.score).then(b.seeders.cmp(&a.seeders)));
}

#[cfg(test)]
pub fn merge_and_rank(hits: Vec<Hit>, filter: &Filter) -> Vec<Torrent> {
    let mut out = merge(hits, 0);
    rank(&mut out, &Ranking { filter, mode: None });
    out
}

// ---------- filters and facets ----------

/// Filter dimensions with a value list (`?language=de,en&res=1080`): OR inside one, AND between them.
pub const DIMENSIONS: &[&str] = &["language", "sub", "res", "source", "codec", "hdr", "audio", "channels", "group"];

#[derive(Debug, Default, Clone, PartialEq)]
pub struct Query {
    /// dimension → wanted values (lowercase).
    pub values: BTreeMap<&'static str, Vec<String>>,
    pub min_seeders: u32,
    /// Bytes; 0 = no limit.
    pub max_size: u64,
    pub min_size: u64,
    pub exclude_cam: bool,
}

impl Query {
    /// Reads the filter parameters of a search request. `exclude_cam` is on unless `exclude_cam=false|0`.
    pub fn from_params(q: &HashMap<String, String>) -> Result<Self, String> {
        let mut out = Self { exclude_cam: !q.get("exclude_cam").is_some_and(|v| matches!(v.as_str(), "false" | "0")), ..Self::default() };
        for dim in DIMENSIONS {
            if let Some(v) = q.get(*dim) {
                let list: Vec<String> = v.split(',').map(|x| x.trim().to_lowercase()).filter(|x| !x.is_empty()).collect();
                if !list.is_empty() {
                    out.values.insert(dim, list);
                }
            }
        }
        let number = |key: &str| -> Result<u64, String> {
            q.get(key).map_or(Ok(0), |v| v.parse().map_err(|_| format!("'{key}' must be a non-negative number")))
        };
        out.min_seeders = number("min_seeders")?.min(u64::from(u32::MAX)) as u32;
        out.max_size = number("max_size")?;
        out.min_size = number("min_size")?;
        Ok(out)
    }
}

/// The values a torrent has in one filter dimension (as shown in the facets).
pub fn facet_values(t: &Torrent, dim: &str) -> Vec<String> {
    let r = &t.release;
    let one = |v: Option<&str>| vec![v.unwrap_or("unknown").to_owned()];
    match dim {
        // "und" = no language stated in the name (by scene convention usually English).
        "language" if r.languages.is_empty() => vec!["und".into()],
        // "dl" = German plus the original ("German DL"), a value of its own besides "de".
        "language" => {
            let mut out: Vec<String> = r.languages.iter().map(|l| l.to_string()).collect();
            if r.dual && r.languages.contains(&"de") {
                out.push("dl".into());
            }
            out
        }
        "sub" => r.subtitles.iter().map(|l| l.to_string()).collect(),
        "res" => one(match r.height {
            Some(h) if h >= 2160 => Some("2160"),
            Some(1080) => Some("1080"),
            Some(720) => Some("720"),
            Some(_) => Some("sd"),
            None => None,
        }),
        "source" => one(r.source),
        "codec" => one(r.codec),
        "hdr" if r.hdr_all.is_empty() => vec!["SDR".into()],
        "hdr" => r.hdr_all.iter().map(|h| h.to_string()).collect(),
        "audio" => one(r.audio),
        "channels" => r.channels.iter().map(|c| c.to_string()).collect(),
        "group" => r.group.iter().cloned().collect(),
        _ => Vec::new(),
    }
}

fn passes(t: &Torrent, q: &Query, skip: Option<&str>) -> bool {
    if q.exclude_cam && t.release.cam {
        return false;
    }
    if t.seeders < q.min_seeders || (q.max_size > 0 && t.size > q.max_size) || t.size < q.min_size {
        return false;
    }
    q.values.iter().filter(|(dim, _)| Some(**dim) != skip).all(|(dim, wanted)| {
        facet_values(t, dim).iter().any(|v| wanted.contains(&v.to_lowercase()))
    })
}

/// value → number of results, per dimension. Each dimension is counted with all *other* filters
/// applied, so the numbers say what choosing that value would show.
pub type Facets = BTreeMap<&'static str, BTreeMap<String, usize>>;

/// Applies the filters; returns the matching torrents (order kept) and the facet counts.
pub fn filter_with_facets(list: &[Torrent], q: &Query) -> (Vec<Torrent>, Facets) {
    let mut facets = Facets::new();
    for dim in DIMENSIONS {
        let counts = facets.entry(dim).or_default();
        for t in list.iter().filter(|t| passes(t, q, Some(dim))) {
            for v in facet_values(t, dim) {
                *counts.entry(v).or_default() += 1;
            }
        }
    }
    // Release groups: only the 30 most common ones.
    if let Some(groups) = facets.get_mut("group")
        && groups.len() > 30
    {
        let mut top: Vec<(String, usize)> = std::mem::take(groups).into_iter().collect();
        top.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
        *groups = top.into_iter().take(30).collect();
    }
    // How many CAM/TS/screener copies the other filters would show (hidden by default).
    let no_cam = Query { exclude_cam: false, ..q.clone() };
    let cam = list.iter().filter(|t| t.release.cam && passes(t, &no_cam, None)).count();
    facets.entry("cam").or_default().insert("true".into(), cam);
    (list.iter().filter(|t| passes(t, q, None)).cloned().collect(), facets)
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
        assert_eq!((r.group.as_deref(), r.dual, r.bit_depth), (Some("NIMA4K"), true, Some(10)));
        assert!(!r.pack && r.episode.is_none());
    }

    #[test]
    fn parses_episode_pack_and_multi() {
        let e = parse("The.Expanse.S03E07.MULTi.1080p.WEB-DL.DD5.1.H.264-GRP");
        assert_eq!((e.title.as_str(), e.season, e.episode, e.pack), ("the expanse", Some(3), Some(7), false));
        assert_eq!((e.source, e.codec, e.audio, e.languages.as_slice()), (Some("WEB-DL"), Some("H.264"), Some("AC3"), &["multi"][..]));
        assert_eq!((e.channels, e.group.as_deref()), (Some("5.1"), Some("GRP")));

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
        assert!(r.languages.is_empty() && !r.dual && r.group.is_none());
        assert_eq!((r.title.as_str(), r.audio, r.source, r.channels), ("sintel", Some("DTS-HD MA"), Some("WEB-DL"), Some("5.1")));
        let t = parse("Web.of.Lies.2023.720p.HDTV.x264");
        assert_eq!((t.title.as_str(), t.source), ("web of lies", Some("HDTV")));
        let y = parse("2001.A.Space.Odyssey.1968.2160p.DV.HDR10+.TrueHD.Atmos");
        assert_eq!((y.title.as_str(), y.year, y.hdr, y.audio), ("2001 a space odyssey", Some(1968), Some("DV"), Some("Atmos")));
        assert_eq!(y.hdr_all, vec!["DV", "HDR10+"]);
    }

    /// Expected fields of one release name; `None`/empty = must be absent.
    #[derive(Default)]
    struct Want {
        title: &'static str,
        height: Option<u32>,
        source: Option<&'static str>,
        codec: Option<&'static str>,
        hdr: &'static [&'static str],
        audio: Option<&'static str>,
        channels: Option<&'static str>,
        langs: &'static [&'static str],
        subs: &'static [&'static str],
        group: Option<&'static str>,
    }

    fn check(name: &str, w: Want) -> Release {
        let r = parse(name);
        assert_eq!(r.title, w.title, "title of {name}");
        assert_eq!(r.height, w.height, "height of {name}");
        assert_eq!(r.source, w.source, "source of {name}");
        assert_eq!(r.codec, w.codec, "codec of {name}");
        assert_eq!(r.hdr_all, w.hdr, "hdr of {name}");
        assert_eq!(r.audio, w.audio, "audio of {name}");
        assert_eq!(r.channels, w.channels, "channels of {name}");
        assert_eq!(r.languages, w.langs, "languages of {name}");
        assert_eq!(r.subtitles, w.subs, "subtitles of {name}");
        assert_eq!(r.group.as_deref(), w.group, "group of {name}");
        r
    }

    #[test]
    fn parses_real_world_release_names() {
        let d = Want::default;
        // --- German scene ---
        let r = check("Oppenheimer.2023.German.DL.1080p.BluRay.x264-DETAiLS", Want { title: "oppenheimer", height: Some(1080), source: Some("BluRay"), codec: Some("H.264"), langs: &["de", "en"], group: Some("DETAiLS"), ..d() });
        assert!(r.dual && r.year == Some(2023));
        check("Dune.Part.Two.2024.German.DL.2160p.UHD.BluRay.DV.HDR10.HEVC.TrueHD.Atmos.7.1.Remux-QfG", Want { title: "dune part two", height: Some(2160), source: Some("Remux"), codec: Some("HEVC"), hdr: &["DV", "HDR10"], audio: Some("Atmos"), channels: Some("7.1"), langs: &["de", "en"], group: Some("QfG"), ..d() });
        let r = check("Barbie.2023.German.AC3D.DL.1080p.WEB.H264-ZeroTwo", Want { title: "barbie", height: Some(1080), source: Some("WEB-DL"), codec: Some("H.264"), audio: Some("AC3"), langs: &["de", "en"], group: Some("ZeroTwo"), ..d() });
        assert_eq!(r.dub, Some("dubbed"));
        let r = check("Deadpool.und.Wolverine.2024.German.MD.TS.1080p.x264-FSX", Want { title: "deadpool und wolverine", height: Some(1080), source: Some("TS"), codec: Some("H.264"), langs: &["de"], group: Some("FSX"), ..d() });
        assert!(r.cam && r.dub == Some("mic"));
        let r = check("Der.Schuh.des.Manitu.2001.GERMAN.1080p.BluRay.x265.AAC5.1-LizardSquad", Want { title: "der schuh des manitu", height: Some(1080), source: Some("BluRay"), codec: Some("HEVC"), audio: Some("AAC"), channels: Some("5.1"), langs: &["de"], group: Some("LizardSquad"), ..d() });
        assert_eq!(r.year, Some(2001));
        let r = check("Dark.S01E03.German.DL.DUBBED.1080p.WebHD.x264-TVP", Want { title: "dark", height: Some(1080), codec: Some("H.264"), langs: &["de", "en"], group: Some("TVP"), ..d() });
        assert_eq!((r.season, r.episode, r.dub), (Some(1), Some(3), Some("dubbed")));
        check("Babylon.Berlin.S04.GERMAN.720p.HDTV.x264-ACED", Want { title: "babylon berlin", height: Some(720), source: Some("HDTV"), codec: Some("H.264"), langs: &["de"], group: Some("ACED"), ..d() });
        let r = check("Das.Boot.1981.Directors.Cut.German.DTS.1080p.BluRay.x264-HDS", Want { title: "das boot", height: Some(1080), source: Some("BluRay"), codec: Some("H.264"), audio: Some("DTS"), langs: &["de"], group: Some("HDS"), ..d() });
        assert_eq!(r.edition, Some("Director's Cut"));
        check("Tatort.E1234.Borowski.GERMAN.1080p.WEB.H264-TSCC", Want { title: "tatort e1234 borowski", height: Some(1080), source: Some("WEB-DL"), codec: Some("H.264"), langs: &["de"], group: Some("TSCC"), ..d() });
        let r = check("One.Piece.E1071.German.Subbed.1080p.WEB.H264-ANiME", Want { title: "one piece e1071", height: Some(1080), source: Some("WEB-DL"), codec: Some("H.264"), subs: &["de"], group: Some("ANiME"), ..d() });
        assert!(r.languages.is_empty());
        check("Stromberg.Der.Film.2014.German.AC3.BDRip.XviD-CiNEDOME", Want { title: "stromberg der film", source: Some("BluRay"), codec: Some("XviD"), audio: Some("AC3"), langs: &["de"], group: Some("CiNEDOME"), ..d() });
        check("Der.Herr.der.Ringe.Die.Gefaehrten.2001.EXTENDED.German.DL.2160p.UHD.BluRay.HDR.x265-ENDSTATiON", Want { title: "der herr der ringe die gefaehrten", height: Some(2160), source: Some("BluRay"), codec: Some("HEVC"), hdr: &["HDR"], langs: &["de", "en"], group: Some("ENDSTATiON"), ..d() });
        check("Dark.GERMAN.1080p.WEB.x264-WAYNE", Want { title: "dark", height: Some(1080), source: Some("WEB-DL"), codec: Some("H.264"), langs: &["de"], group: Some("WAYNE"), ..d() });
        check("Inception.German.DL.1080p.BluRay.x264-RSG", Want { title: "inception", height: Some(1080), source: Some("BluRay"), codec: Some("H.264"), langs: &["de", "en"], group: Some("RSG"), ..d() });
        check("Avatar.The.Way.of.Water.2022.German.EAC3D.DL.2160p.DV.HDR.WEB.H265-ZeroTwo", Want { title: "avatar the way of water", height: Some(2160), source: Some("WEB-DL"), codec: Some("HEVC"), hdr: &["DV", "HDR"], audio: Some("EAC3"), langs: &["de", "en"], group: Some("ZeroTwo"), ..d() });
        let r = check("Joker.Folie.a.Deux.2024.German.LD.HDCAM.x264-SnD", Want { title: "joker folie a deux", source: Some("CAM"), codec: Some("H.264"), langs: &["de"], group: Some("SnD"), ..d() });
        assert!(r.cam && r.dub == Some("line"));
        check("Die.Discounter.S03.COMPLETE.GERMAN.1080p.AMZN.WEB-DL.DDP5.1.H.264-MGE", Want { title: "die discounter", height: Some(1080), source: Some("WEB-DL"), codec: Some("H.264"), audio: Some("EAC3"), channels: Some("5.1"), langs: &["de"], group: Some("MGE"), ..d() });
        check("Shogun.2024.S01E05.German.DL.EAC3.1080p.DSNP.WEB.H265-ZeroTwo", Want { title: "shogun", height: Some(1080), source: Some("WEB-DL"), codec: Some("HEVC"), audio: Some("EAC3"), langs: &["de", "en"], group: Some("ZeroTwo"), ..d() });
        check("The.Good.German.2006.1080p.BluRay.x264-HD4U", Want { title: "the good german", height: Some(1080), source: Some("BluRay"), codec: Some("H.264"), group: Some("HD4U"), ..d() });
        check("Das.Leben.der.Anderen.2006.German.ML.Complete.PAL.DVD9-iNri", Want { title: "das leben der anderen", source: Some("DVD"), langs: &["de", "multi"], group: Some("iNri"), ..d() });

        // --- international ---
        check("The.Matrix.1999.2160p.UHD.BluRay.x265.10bit.HDR.TrueHD.7.1.Atmos-SWTYBLZ", Want { title: "the matrix", height: Some(2160), source: Some("BluRay"), codec: Some("HEVC"), hdr: &["HDR"], audio: Some("Atmos"), channels: Some("7.1"), group: Some("SWTYBLZ"), ..d() });
        let r = check("Blade.Runner.2049.2017.1080p.BluRay.DTS-HD.MA.7.1.x264-FGT", Want { title: "blade runner 2049", height: Some(1080), source: Some("BluRay"), codec: Some("H.264"), audio: Some("DTS-HD MA"), channels: Some("7.1"), group: Some("FGT"), ..d() });
        assert_eq!(r.year, Some(2017));
        check("Severance.S02E01.1080p.ATVP.WEB-DL.DDP5.1.Atmos.H.264-FLUX", Want { title: "severance", height: Some(1080), source: Some("WEB-DL"), codec: Some("H.264"), audio: Some("Atmos"), channels: Some("5.1"), group: Some("FLUX"), ..d() });
        check("The.Bear.S03E01.720p.HDTV.x264-SYNCOPY[rarbg]", Want { title: "the bear", height: Some(720), source: Some("HDTV"), codec: Some("H.264"), group: Some("SYNCOPY"), ..d() });
        check("Interstellar (2014) [2160p] [4K] [BluRay] [5.1] [YTS.MX]", Want { title: "interstellar", height: Some(2160), source: Some("BluRay"), channels: Some("5.1"), group: Some("YTS.MX"), ..d() });
        let r = check("Spider-Man.No.Way.Home.2021.PROPER.1080p.WEBRip.x265.10bit.AAC5.1-GalaxyRG", Want { title: "spider man no way home", height: Some(1080), source: Some("WEBRip"), codec: Some("HEVC"), audio: Some("AAC"), channels: Some("5.1"), group: Some("GalaxyRG"), ..d() });
        assert!(r.proper && r.bit_depth == Some(10));
        let r = check("Top.Gun.Maverick.2022.IMAX.REPACK.2160p.WEB-DL.DDP5.1.Atmos.DV.HDR10+.H.265-CMRG", Want { title: "top gun maverick", height: Some(2160), source: Some("WEB-DL"), codec: Some("HEVC"), hdr: &["DV", "HDR10+"], audio: Some("Atmos"), channels: Some("5.1"), group: Some("CMRG"), ..d() });
        assert!(r.repack && r.edition == Some("IMAX"));
        check("Le.Fabuleux.Destin.d.Amelie.Poulain.2001.MULTi.1080p.BluRay.x264-LOST", Want { title: "le fabuleux destin d amelie poulain", height: Some(1080), source: Some("BluRay"), codec: Some("H.264"), langs: &["multi"], group: Some("LOST"), ..d() });
        check("Parasite.2019.VOSTFR.1080p.WEB-DL.H264-Slay3R", Want { title: "parasite", height: Some(1080), source: Some("WEB-DL"), codec: Some("H.264"), subs: &["fr"], group: Some("Slay3R"), ..d() });
        check("La.Casa.de.Papel.S01E01.SPANISH.1080p.NF.WEB-DL.DD5.1.x264-NTG", Want { title: "la casa de papel", height: Some(1080), source: Some("WEB-DL"), codec: Some("H.264"), audio: Some("AC3"), channels: Some("5.1"), langs: &["es"], group: Some("NTG"), ..d() });
        check("La.Vita.e.Bella.1997.iTA.ENG.1080p.BluRay.x264-MIRCrew", Want { title: "la vita e bella", height: Some(1080), source: Some("BluRay"), codec: Some("H.264"), langs: &["en", "it"], group: Some("MIRCrew"), ..d() });
        check("Squid.Game.S02E01.KOREAN.1080p.NF.WEB-DL.DDP5.1.Atmos.H.264.ESubs-XEBEC", Want { title: "squid game", height: Some(1080), source: Some("WEB-DL"), codec: Some("H.264"), audio: Some("Atmos"), channels: Some("5.1"), langs: &["ko"], subs: &["en"], group: Some("XEBEC"), ..d() });
        check("The.English.Patient.1996.1080p.BluRay.x264.DTS-FGT", Want { title: "the english patient", height: Some(1080), source: Some("BluRay"), codec: Some("H.264"), audio: Some("DTS"), group: Some("FGT"), ..d() });
        check("The.Italian.Job.1969.720p.BluRay.x264.AAC-YTS", Want { title: "the italian job", height: Some(720), source: Some("BluRay"), codec: Some("H.264"), audio: Some("AAC"), group: Some("YTS"), ..d() });
        let r = check("Gladiator.II.2024.1080p.HDTS.x264.AAC-NoGroup", Want { title: "gladiator ii", height: Some(1080), source: Some("TS"), codec: Some("H.264"), audio: Some("AAC"), group: Some("NoGroup"), ..d() });
        assert!(r.cam);
        assert!(check("Wicked.2024.HDCAM.c1nem4.x264-SUNSCREEN", Want { title: "wicked", source: Some("CAM"), codec: Some("H.264"), group: Some("SUNSCREEN"), ..d() }).cam);
        assert!(check("Some.Movie.2023.TC.1080p.x264.AC3-EVO", Want { title: "some movie", height: Some(1080), source: Some("TC"), codec: Some("H.264"), audio: Some("AC3"), group: Some("EVO"), ..d() }).cam);
        assert!(check("Another.Film.2024.DVDScr.XviD.AC3-HQ", Want { title: "another film", source: Some("SCR"), codec: Some("XviD"), audio: Some("AC3"), group: Some("HQ"), ..d() }).cam);
        check("Cam.2018.1080p.NF.WEB-DL.DD5.1.H264-NTG", Want { title: "cam", height: Some(1080), source: Some("WEB-DL"), codec: Some("H.264"), audio: Some("AC3"), channels: Some("5.1"), group: Some("NTG"), ..d() });
        check("Akira.1988.JAPANESE.2160p.UHD.BluRay.HDR10.HEVC.TrueHD.5.1-KRaLiMaRKo", Want { title: "akira", height: Some(2160), source: Some("BluRay"), codec: Some("HEVC"), hdr: &["HDR10"], audio: Some("TrueHD"), channels: Some("5.1"), langs: &["ja"], group: Some("KRaLiMaRKo"), ..d() });
        check("[SubsPlease] Frieren - 12 (1080p) [ABCD1234].mkv", Want { title: "subsplease frieren 12", height: Some(1080), group: Some("ABCD1234"), ..d() });
        let r = check("Demon.Slayer.S01.1080p.BluRay.Dual.Audio.FLAC.x265-EMBER", Want { title: "demon slayer", height: Some(1080), source: Some("BluRay"), codec: Some("HEVC"), audio: Some("FLAC"), langs: &["multi"], group: Some("EMBER"), ..d() });
        assert!(r.dual && r.pack);
        check("Planet.Earth.II.2016.S01E01.2160p.UHD.BluRay.HLG.DTS-HD.MA.5.1.HEVC.REMUX-FraMeSToR", Want { title: "planet earth ii", height: Some(2160), source: Some("Remux"), codec: Some("HEVC"), hdr: &["HLG"], audio: Some("DTS-HD MA"), channels: Some("5.1"), group: Some("FraMeSToR"), ..d() });
        check("Sintel.2010.1080p.AV1.Opus.2.0-GRP.mkv", Want { title: "sintel", height: Some(1080), codec: Some("AV1"), audio: Some("Opus"), channels: Some("2.0"), group: Some("GRP"), ..d() });
        let r = check("Avatar.2009.3D.HSBS.1080p.BluRay.x264.DTS-HD", Want { title: "avatar", height: Some(1080), source: Some("BluRay"), codec: Some("H.264"), audio: Some("DTS-HD"), ..d() });
        assert!(r.three_d);
        check("Big.Buck.Bunny.2008.1920x1080.H264.AAC.mp4", Want { title: "big buck bunny", height: Some(1080), codec: Some("H.264"), audio: Some("AAC"), ..d() });
        check("The.Office.US.S05E13.NORDiC.720p.WEB-DL.H.264.ENG.SUBS-TWA", Want { title: "the office us", height: Some(720), source: Some("WEB-DL"), codec: Some("H.264"), langs: &["nordic"], subs: &["en"], group: Some("TWA"), ..d() });
        check("Tears.of.Steel.2012.4K.WEBRip.2160p.HEVC.10bit.DD+5.1-Mango", Want { title: "tears of steel", height: Some(2160), source: Some("WEBRip"), codec: Some("HEVC"), audio: Some("EAC3"), channels: Some("5.1"), group: Some("Mango"), ..d() });
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
    fn keeps_the_earliest_upload_time() {
        let mut a = hit("a", "aa", "Movie 2020 1080p", 1, &[]);
        a.added = Some(86_400 * 10);
        let mut b = hit("b", "aa", "Movie 2020 1080p", 1, &[]);
        b.added = Some(86_400 * 4);
        let out = merge(vec![a, b, hit("c", "aa", "Movie 2020 1080p", 1, &[])], 86_400 * 14);
        assert_eq!((out[0].added, out[0].age_days), (Some(86_400 * 4), Some(10)));
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

    #[test]
    fn fast_start_prefers_seeders_per_size_and_quality_prefers_the_remux() {
        let sized = |hash: &str, name: &str, seeders: u32, gb: u64| Hit { size: gb * 1_000_000_000, ..hit("a", hash, name, seeders, &[]) };
        let hits = || {
            vec![
                sized("remux", "Film.2020.2160p.UHD.BluRay.HDR.HEVC.Remux-A", 40, 60),
                sized("web", "Film.2020.1080p.WEB-DL.x264-B", 900, 4),
                sized("small", "Film.2020.720p.WEBRip.x264-C", 300, 1),
            ]
        };
        let f = filter(0, "any", "quality");
        let order = |mode: &str| {
            let mut list = merge(hits(), 0);
            rank(&mut list, &Ranking { filter: &f, mode: Some(mode) });
            list.into_iter().map(|t| t.infohash).collect::<Vec<_>>()
        };
        assert_eq!(order("fast"), ["web", "small", "remux"]);
        assert_eq!(order("quality")[0], "remux");
    }

    #[test]
    fn filters_and_counts_facets() {
        let list = merge(
            vec![
                hit("a", "1", "Film.2020.German.DL.1080p.BluRay.x264-AAA", 50, &[]),
                hit("a", "2", "Film.2020.German.1080p.WEB.H264-BBB", 30, &[]),
                hit("a", "3", "Film.2020.German.DL.2160p.UHD.BluRay.HDR.HEVC.Remux-AAA", 10, &[]),
                hit("a", "4", "Film.2020.1080p.BluRay.x264-CCC", 400, &[]),
                hit("a", "5", "Film.2020.720p.WEBRip.x264-CCC", 200, &[]),
                hit("a", "6", "Film.2020.German.MD.TS.x264-DDD", 900, &[]),
                Hit { size: 50_000_000_000, ..hit("a", "7", "Film.2020.German.DL.1080p.BluRay.AVC.Remux-EEE", 5, &[]) },
            ],
            0,
        );
        let query = |pairs: &[(&str, &str)]| Query::from_params(&pairs.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect()).unwrap();
        let ids = |q: &Query| filter_with_facets(&list, q).0.iter().map(|t| t.infohash.clone()).collect::<Vec<_>>().join("");

        // No filter: everything but the telesync.
        let (all, facets) = filter_with_facets(&list, &query(&[]));
        assert_eq!(all.len(), 6);
        assert_eq!(facets["cam"]["true"], 1);
        assert_eq!((facets["language"]["de"], facets["language"]["en"], facets["language"]["und"]), (4, 3, 2));
        assert_eq!((facets["res"]["1080"], facets["res"]["2160"], facets["res"]["720"]), (4, 1, 1));
        assert_eq!((facets["source"]["BluRay"], facets["source"]["Remux"], facets["source"]["WEB-DL"], facets["source"]["WEBRip"]), (2, 2, 1, 1));
        assert_eq!((facets["hdr"]["SDR"], facets["hdr"]["HDR"]), (5, 1));
        assert_eq!(facets["group"]["AAA"], 2);

        // "Deutsch + 1080p + BluRay": only the matching release; each facet counts with the other filters applied.
        let q = query(&[("language", "de"), ("res", "1080"), ("source", "bluray")]);
        let (hits, facets) = filter_with_facets(&list, &q);
        assert_eq!(hits.iter().map(|t| t.infohash.as_str()).collect::<Vec<_>>(), ["1"]);
        assert_eq!((facets["language"]["de"], facets["language"]["und"]), (1, 1));
        assert_eq!((facets["source"]["BluRay"], facets["source"]["WEB-DL"], facets["source"]["Remux"]), (1, 1, 1));
        assert_eq!(facets["res"]["1080"], 1);
        assert!(!facets["res"].contains_key("2160"));

        assert_eq!(ids(&query(&[("source", "bluray,remux"), ("language", "de")])), "137");
        assert_eq!(ids(&query(&[("language", "de"), ("max_size", "10000000000")])), "123");
        assert_eq!(ids(&query(&[("min_seeders", "100")])), "45");
        assert_eq!(ids(&query(&[("min_seeders", "100"), ("exclude_cam", "false")])), "456");
        assert_eq!(ids(&query(&[("hdr", "hdr"), ("codec", "hevc")])), "3");
        assert_eq!(ids(&query(&[("group", "ccc")])), "45");
        assert!(Query::from_params(&[("max_size".to_string(), "big".to_string())].into()).is_err());
    }

    #[test]
    fn unknown_seeders_rank_between_live_and_dead_and_source_languages_count() {
        let unknown = Hit { no_seeds: true, languages: vec!["de"], ..hit("zilean", "1", "Film.2020.1080p.BluRay.x264-AAA", 0, &[]) };
        let dead = hit("a", "2", "Film.2020.1080p.BluRay.x264-BBB", 0, &[]);
        let live = hit("a", "3", "Film.2020.720p.WEB.x264-CCC", 2, &[]);
        let out = merge_and_rank(vec![dead, unknown, live], &filter(0, "any", "quality"));
        assert_eq!(out.iter().map(|t| t.infohash.as_str()).collect::<Vec<_>>(), ["3", "1", "2"]);
        assert!(out[1].seeders_unknown && !out[2].seeders_unknown);
        assert_eq!(out[1].release.languages, ["de"]);
        // A source with numbers makes the merged torrent "known".
        let both = merge(vec![Hit { no_seeds: true, ..hit("x", "9", "F", 0, &[]) }, hit("y", "9", "F", 4, &[])], 0);
        assert!(!both[0].seeders_unknown);
        assert_eq!(languages_in("English + Russian"), ["en", "ru"]);
        let dl = merge(vec![hit("a", "8", "Film.2020.German.DL.1080p.BluRay.x264-AAA", 3, &[])], 0);
        assert_eq!(facet_values(&dl[0], "language"), ["de", "en", "dl"]);
    }
}
