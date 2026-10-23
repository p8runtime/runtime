//! Input encodings: what libxml2 converts a document from (to UTF-8)
//! when its XML declaration or BOM names one.

/// A converter to UTF-8.
#[derive(Clone, Copy)]
pub(crate) enum Decoder {
    Utf16Le,
    Utf16Be,
    Latin1,
    Ascii,
    Other(&'static encoding_rs::Encoding),
}

/// The decoder for an encoding name (None: libxml2 would not know it).
pub(crate) fn decoder(name: &[u8]) -> Option<Decoder> {
    let upper = name.to_ascii_uppercase();
    Some(match upper.as_slice() {
        b"UTF-16LE" | b"UTF16LE" => Decoder::Utf16Le,
        b"UTF-16BE" | b"UTF16BE" => Decoder::Utf16Be,
        b"ISO-8859-1" | b"ISO-LATIN-1" | b"ISO LATIN 1" | b"LATIN1" | b"ISO8859-1" | b"ISO_8859-1" | b"L1" => Decoder::Latin1,
        b"US-ASCII" | b"ASCII" => Decoder::Ascii,
        _ => Decoder::Other(encoding_rs::Encoding::for_label_no_replacement(name)?),
    })
}

impl Decoder {
    pub(crate) fn decode(self, bytes: &[u8]) -> Vec<u8> {
        match self {
            Decoder::Utf16Le | Decoder::Utf16Be => {
                let units: Vec<u16> = bytes
                    .chunks(2)
                    .filter(|c| c.len() == 2)
                    .map(|c| if matches!(self, Decoder::Utf16Le) { u16::from_le_bytes([c[0], c[1]]) } else { u16::from_be_bytes([c[0], c[1]]) })
                    .collect();
                String::from_utf16_lossy(&units).into_bytes()
            }
            Decoder::Latin1 | Decoder::Ascii => bytes.iter().map(|&b| char::from(b)).collect::<String>().into_bytes(),
            Decoder::Other(e) => e.decode_without_bom_handling(bytes).0.into_owned().into_bytes(),
        }
    }
}
