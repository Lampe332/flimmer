//! Reads the codecs of a video file from its container headers, for hosts without ffprobe (Android).
//! MP4/MOV: the sample entries (`stsd`) and the movie header (`mvhd`). Matroska/WebM: the codec ids of the tracks.
//! Enough to decide whether the built-in player can play the file; no sizes, no languages, no subtitles.

/// What the headers say.
#[derive(Debug, Default, PartialEq)]
pub struct Codecs {
    /// ffprobe's name of the first video codec.
    pub video: Option<String>,
    /// ffprobe's names of the audio codecs, in file order.
    pub audio: Vec<String>,
    pub duration: Option<f64>,
}

const MP4_VIDEO: &[(&[u8; 4], &str)] = &[
    (b"avc1", "h264"),
    (b"avc3", "h264"),
    (b"hvc1", "hevc"),
    (b"hev1", "hevc"),
    (b"dvh1", "hevc"),
    (b"dvhe", "hevc"),
    (b"vp09", "vp9"),
    (b"vp08", "vp8"),
    (b"av01", "av1"),
    (b"mp4v", "mpeg4"),
];
const MP4_AUDIO: &[(&[u8; 4], &str)] = &[
    (b"mp4a", "aac"),
    (b"ac-3", "ac3"),
    (b"ec-3", "eac3"),
    (b"dtsc", "dts"),
    (b"dtsh", "dts"),
    (b"dtsl", "dts"),
    (b"dtse", "dts"),
    (b"Opus", "opus"),
    (b"fLaC", "flac"),
    (b".mp3", "mp3"),
    (b"mlpa", "truehd"),
];
const MKV_VIDEO: &[(&str, &str)] = &[
    ("V_MPEG4/ISO/AVC", "h264"),
    ("V_MPEGH/ISO/HEVC", "hevc"),
    ("V_VP9", "vp9"),
    ("V_VP8", "vp8"),
    ("V_AV1", "av1"),
    ("V_MPEG4/ISO/ASP", "mpeg4"),
    ("V_MPEG2", "mpeg2video"),
];
const MKV_AUDIO: &[(&str, &str)] = &[
    ("A_AAC", "aac"),
    ("A_EAC3", "eac3"),
    ("A_AC3", "ac3"),
    ("A_DTS", "dts"),
    ("A_OPUS", "opus"),
    ("A_VORBIS", "vorbis"),
    ("A_FLAC", "flac"),
    ("A_MPEG/L3", "mp3"),
    ("A_TRUEHD", "truehd"),
];

fn positions(hay: &[u8], needle: &[u8]) -> Vec<usize> {
    if needle.is_empty() || hay.len() < needle.len() {
        return Vec::new();
    }
    hay.windows(needle.len()).enumerate().filter(|(_, w)| *w == needle).map(|(i, _)| i).collect()
}

fn be32(b: &[u8], at: usize) -> Option<u64> {
    b.get(at..at + 4).map(|s| u32::from_be_bytes([s[0], s[1], s[2], s[3]]) as u64)
}

fn be64(b: &[u8], at: usize) -> Option<u64> {
    b.get(at..at + 8).map(|s| u64::from_be_bytes(s.try_into().expect("8 bytes")))
}

fn mp4(part: &[u8], out: &mut Codecs) {
    // `stsd`: tag, version+flags, entry count, then the first entry: size, format.
    for at in positions(part, b"stsd") {
        let Some(fourcc) = part.get(at + 16..at + 20) else { continue };
        if let Some((_, name)) = MP4_VIDEO.iter().find(|(f, _)| f.as_slice() == fourcc) {
            out.video.get_or_insert_with(|| (*name).to_owned());
        } else if let Some((_, name)) = MP4_AUDIO.iter().find(|(f, _)| f.as_slice() == fourcc) {
            out.audio.push((*name).to_owned());
        }
    }
    if out.duration.is_none()
        && let Some(at) = positions(part, b"mvhd").first().copied()
    {
        let (scale, units) = match part.get(at + 4) {
            Some(1) => (be32(part, at + 24), be64(part, at + 28)),
            _ => (be32(part, at + 16), be32(part, at + 20)),
        };
        if let (Some(scale), Some(units)) = (scale, units)
            && scale > 0
        {
            out.duration = Some(units as f64 / scale as f64);
        }
    }
}

fn matroska(head: &[u8], out: &mut Codecs) {
    out.video = MKV_VIDEO
        .iter()
        .filter_map(|(id, name)| positions(head, id.as_bytes()).first().map(|at| (*at, *name)))
        .min_by_key(|(at, _)| *at)
        .map(|(_, name)| name.to_owned());
    let mut audio: Vec<(usize, &str)> = Vec::new();
    for (id, name) in MKV_AUDIO {
        audio.extend(positions(head, id.as_bytes()).into_iter().map(|at| (at, *name)));
    }
    audio.sort_by_key(|(at, _)| *at);
    out.audio = audio.into_iter().map(|(_, name)| name.to_owned()).collect();
}

/// `head`: the file's first bytes, `tail`: its last ones (MP4 files not made for streaming keep `moov` there).
pub fn read(ext: &str, head: &[u8], tail: &[u8]) -> Codecs {
    let mut out = Codecs::default();
    match ext {
        "mkv" | "webm" => matroska(head, &mut out),
        _ => {
            mp4(head, &mut out);
            if out.video.is_none() && out.audio.is_empty() {
                mp4(tail, &mut out);
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn stsd(fourcc: &[u8; 4]) -> Vec<u8> {
        let mut b = vec![0, 0, 0, 100];
        b.extend_from_slice(b"stsd");
        b.extend_from_slice(&[0, 0, 0, 0, 0, 0, 0, 1, 0, 0, 0, 84]);
        b.extend_from_slice(fourcc);
        b.extend_from_slice(&[0; 40]);
        b
    }

    #[test]
    fn mp4_codecs_and_duration() {
        let mut moov = vec![0u8; 8];
        moov.extend_from_slice(b"mvhd");
        moov.extend_from_slice(&[0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0]);
        moov.extend_from_slice(&1000u32.to_be_bytes());
        moov.extend_from_slice(&888_000u32.to_be_bytes());
        moov.extend(stsd(b"avc1"));
        moov.extend(stsd(b"mp4a"));
        moov.extend(stsd(b"ac-3"));
        let c = read("mp4", &moov, &[]);
        assert_eq!(c.video.as_deref(), Some("h264"));
        assert_eq!(c.audio, ["aac", "ac3"]);
        assert_eq!(c.duration, Some(888.0));
        // `moov` at the end of the file
        assert_eq!(read("mp4", b"....mdat....", &moov), c);
        assert_eq!(read("mp4", b"nothing here", b""), Codecs::default());
    }

    #[test]
    fn matroska_codecs_in_track_order() {
        let head = b"..\x86\x90V_MPEGH/ISO/HEVC....\x86\x86A_EAC3....\x86\x85A_AAC..";
        let c = read("mkv", head, &[]);
        assert_eq!(c.video.as_deref(), Some("hevc"));
        assert_eq!(c.audio, ["eac3", "aac"]);
    }
}
