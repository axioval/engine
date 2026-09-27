//! Facts about whole sources: the file a host read a source from, the
//! applications that wrote it, the schema it declares and the project it
//! describes.
//!
//! Metadata is not an object fact and not part of a snapshot's identity:
//! services bound to a snapshot stay valid whatever a host learns about the
//! file. Each field is either unread (`None`), or read completely, holding
//! every distinct value the source states, possibly none. An unread field is
//! unknown, never "no value": a `source` selector over it is not evaluated.

use std::collections::BTreeMap;

use axioval_ir::SourceId;
use axioval_ir::contract::SourceField;

/// What is known about one source.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct SourceMetadata {
    fields: BTreeMap<SourceField, Vec<String>>,
}

impl SourceMetadata {
    /// Metadata with no field read.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// States `field` as read completely, holding `values`.
    ///
    /// Values are kept sorted and distinct; blank ones state nothing and are
    /// dropped, so a field of only blank values is read and empty. Stating a
    /// field again replaces it.
    #[must_use]
    pub fn with<I, V>(mut self, field: SourceField, values: I) -> Self
    where
        I: IntoIterator<Item = V>,
        V: Into<String>,
    {
        let mut values: Vec<String> = values
            .into_iter()
            .map(Into::into)
            .filter(|value| !value.trim().is_empty())
            .collect();
        values.sort();
        values.dedup();
        self.fields.insert(field, values);
        self
    }

    /// The values of `field`, sorted and distinct; `None` when it was never
    /// read.
    #[must_use]
    pub fn values(&self, field: SourceField) -> Option<&[String]> {
        self.fields.get(&field).map(Vec::as_slice)
    }

    /// Every read field with its values.
    pub fn fields(&self) -> impl Iterator<Item = (SourceField, &[String])> {
        self.fields
            .iter()
            .map(|(field, values)| (*field, values.as_slice()))
    }

    /// Both statements together.
    ///
    /// # Errors
    ///
    /// The first field both state with different values: two readers of
    /// one source disagree, and neither is taken.
    pub fn merged(mut self, other: Self) -> Result<Self, SourceField> {
        for (field, values) in other.fields {
            match self.fields.get(&field) {
                Some(held) if *held != values => return Err(field),
                Some(_) => {}
                None => {
                    self.fields.insert(field, values);
                }
            }
        }
        Ok(self)
    }
}

/// The metadata of every source of a run.
///
/// Only the engine constructs this, from the evidence session, and registers
/// it for the duration of one run, replacing any host-registered copy, as it
/// does [`crate::SourceDisciplines`]. A source's schema comes from its
/// snapshot unless the session states it. A source without an entry has no
/// field read.
#[derive(Clone, Debug, Default)]
pub struct SourceMetadataIndex(BTreeMap<SourceId, SourceMetadata>);

impl SourceMetadataIndex {
    /// Indexes `metadata` by source.
    ///
    /// Capability tests construct it; a run always uses the runtime's own.
    pub fn new(metadata: impl IntoIterator<Item = (SourceId, SourceMetadata)>) -> Self {
        Self(metadata.into_iter().collect())
    }

    /// Everything known about `source`, if anything.
    #[must_use]
    pub fn of(&self, source: &SourceId) -> Option<&SourceMetadata> {
        self.0.get(source)
    }

    /// The values of `field` for `source`; `None` when never read.
    #[must_use]
    pub fn values(&self, source: &SourceId, field: SourceField) -> Option<&[String]> {
        self.of(source).and_then(|metadata| metadata.values(field))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn values_are_sorted_distinct_and_never_blank() {
        let metadata = SourceMetadata::new().with(SourceField::Application, ["b", " ", "a", "b"]);
        assert_eq!(
            metadata.values(SourceField::Application),
            Some(&["a".to_owned(), "b".to_owned()][..])
        );
        assert_eq!(metadata.values(SourceField::Project), None);
        let empty = SourceMetadata::new().with(SourceField::Project, [""]);
        assert_eq!(empty.values(SourceField::Project), Some(&[][..]));
    }

    #[test]
    fn merging_refuses_a_field_stated_differently() {
        let adapter = SourceMetadata::new().with(SourceField::Application, ["Tool"]);
        let host = SourceMetadata::new().with(SourceField::FileName, ["a.ifc"]);
        let merged = adapter.clone().merged(host).unwrap();
        assert_eq!(merged.values(SourceField::FileName).unwrap().len(), 1);
        assert_eq!(
            merged.clone().merged(adapter.clone()).unwrap(),
            merged,
            "an identical statement agrees"
        );
        let other = SourceMetadata::new().with(SourceField::Application, ["Other"]);
        assert_eq!(adapter.merged(other), Err(SourceField::Application));
    }
}
