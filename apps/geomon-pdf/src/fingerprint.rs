use crate::{Result, invalid};
use pdfcraft_cos::{Dict, Document, ObjRef, Object};
use pdfcraft_model::Page;
use sha2::{Digest, Sha256};
use std::collections::HashSet;

const REVIEW_TYPES: &[&[u8]] = &[
    b"Text",
    b"FreeText",
    b"Line",
    b"Square",
    b"Circle",
    b"Polygon",
    b"PolyLine",
    b"Highlight",
    b"Underline",
    b"Squiggly",
    b"StrikeOut",
    b"Ink",
    b"Popup",
];

pub fn drawing(document: &Document, pages: &[Page]) -> Result<[u8; 32]> {
    let mut walker = Fingerprint { document, hash: Sha256::new(), seen: HashSet::new(), remaining: 200_000, bytes: 256 * 1024 * 1024 };
    let root = document.root().ok_or_else(|| invalid("PDF catalog missing."))?;
    let catalog = document.get(root);
    let catalog = catalog.as_dict().ok_or_else(|| invalid("PDF catalog is not a dictionary."))?;
    walker.dictionary(catalog, &[b"Pages", b"Metadata"], 0)?;
    for page in pages {
        walker.add(&page.obj.num.to_be_bytes())?;
        walker.add(&page.obj.generation.to_be_bytes())?;
        walker.dictionary(&page.dict, &[b"Parent", b"Annots"], 0)?;
        walker.annotations(page.dict.get(b"Annots"), 0)?;
    }
    Ok(walker.hash.finalize().into())
}

struct Fingerprint<'a> {
    document: &'a Document,
    hash: Sha256,
    seen: HashSet<ObjRef>,
    remaining: usize,
    bytes: usize,
}

impl Fingerprint<'_> {
    fn add(&mut self, bytes: &[u8]) -> Result<()> {
        self.bytes = self.bytes.checked_sub(bytes.len()).ok_or_else(|| invalid("PDF verification byte budget exceeded."))?;
        self.hash.update((bytes.len() as u64).to_be_bytes());
        self.hash.update(bytes);
        Ok(())
    }

    fn dictionary(&mut self, dict: &Dict, skipped: &[&[u8]], depth: usize) -> Result<()> {
        self.add(b"dict")?;
        let mut entries: Vec<_> = dict.iter().filter(|(key, _)| !skipped.contains(&key.as_slice())).collect();
        entries.sort_by_key(|(a, _)| *a);
        for (key, value) in entries {
            self.add(key)?;
            self.object(value, depth + 1)?;
        }
        self.add(b"enddict")
    }

    fn annotations(&mut self, value: Option<&Object>, depth: usize) -> Result<()> {
        self.add(b"non-review-annotations")?;
        let Some(value) = value else {
            return Ok(());
        };
        let value = self.document.resolve(value);
        let array = value.as_array().ok_or_else(|| invalid("PDF annotations could not be verified."))?;
        for value in array {
            let resolved = self.document.resolve(value);
            let dict = resolved.as_dict().ok_or_else(|| invalid("PDF annotation could not be verified."))?;
            let subtype = dict.get(b"Subtype").map(|value| self.document.resolve(value));
            let allowed = subtype.as_deref().and_then(Object::as_name).is_some_and(|name| REVIEW_TYPES.contains(&name));
            if allowed && !dict.contains(b"A") && !dict.contains(b"AA") {
                continue;
            }
            self.object(value, depth + 1)?;
        }
        Ok(())
    }

    fn object(&mut self, value: &Object, depth: usize) -> Result<()> {
        if depth > 128 {
            return Err(invalid("PDF verification graph is too deep."));
        }
        self.remaining = self.remaining.checked_sub(1).ok_or_else(|| invalid("PDF verification object budget exceeded."))?;
        match value {
            Object::Null => self.add(b"null"),
            Object::Bool(value) => self.add(if *value { b"true" } else { b"false" }),
            Object::Int(value) => {
                self.add(b"number")?;
                self.add(value.to_string().as_bytes())
            }
            Object::Real(value) => {
                if !value.is_finite() {
                    return Err(invalid("PDF verification found a non-finite number."));
                }
                self.add(b"number")?;
                self.add(if *value == 0.0 { "0".into() } else { value.to_string() }.as_bytes())
            }
            Object::Name(value) => {
                self.add(b"name")?;
                self.add(value)
            }
            Object::String(value) => {
                self.add(b"string")?;
                self.add(&value.bytes)
            }
            Object::Array(values) => {
                self.add(b"array")?;
                for value in values {
                    self.object(value, depth + 1)?;
                }
                self.add(b"endarray")
            }
            Object::Dict(dict) if dict.name(b"Type") == Some(b"Page") => {
                self.dictionary(dict, &[b"Parent", b"Annots"], depth)?;
                self.annotations(dict.get(b"Annots"), depth)
            }
            Object::Dict(dict) => self.dictionary(dict, &[], depth),
            Object::Stream(stream) => {
                self.add(b"stream")?;
                self.dictionary(&stream.dict, &[b"Length"], depth)?;
                self.add(&stream.raw)
            }
            Object::Ref(reference) => {
                self.add(b"reference")?;
                self.add(&reference.num.to_be_bytes())?;
                self.add(&reference.generation.to_be_bytes())?;
                if self.seen.insert(*reference) {
                    let resolved = self.document.get(*reference);
                    if matches!(resolved.as_ref(), Object::Null) {
                        return Err(invalid("PDF verification found an unresolved object."));
                    }
                    self.object(&resolved, depth + 1)?;
                }
                Ok(())
            }
        }
    }
}
