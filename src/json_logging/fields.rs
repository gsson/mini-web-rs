use crate::json_logging::format::FormatEvent;
use serde::{Serialize, Serializer};
use tracing::field::{Field, Visit};
use tracing::span::{Attributes, Record};
use tracing::Event;

#[derive(Default)]
pub(crate) struct FieldRecorder {
    fields: Vec<(&'static str, RecordedValue)>,
}

impl FieldRecorder {
    pub fn from_event(event_format: &impl FormatEvent, event: &Event) -> FieldRecorder {
        let mut fields = FieldRecorder::default();
        event.record(&mut FieldVisitor::new(event_format, &mut fields));
        fields
    }

    pub fn from_attributes(
        event_format: &impl FormatEvent,
        attributes: &Attributes,
    ) -> FieldRecorder {
        let mut fields = FieldRecorder::default();
        attributes.record(&mut FieldVisitor::new(event_format, &mut fields));
        fields
    }

    pub fn append_record(&mut self, event_format: &impl FormatEvent, record: &Record<'_>) {
        record.record(&mut FieldVisitor::new(event_format, self));
    }

    fn push(&mut self, field: &Field, value: RecordedValue) {
        self.fields.push((field.name(), value));
    }
}

pub struct RecordedFieldsIter<'a> {
    inner: std::iter::Rev<std::slice::Iter<'a, (&'static str, RecordedValue)>>,
}

impl<'a> IntoIterator for &'a FieldRecorder {
    type Item = &'a (&'static str, RecordedValue);
    type IntoIter = RecordedFieldsIter<'a>;

    fn into_iter(self) -> Self::IntoIter {
        Self::IntoIter {
            inner: self.fields.iter().rev(),
        }
    }
}

impl<'a> Iterator for RecordedFieldsIter<'a> {
    type Item = &'a (&'static str, RecordedValue);

    fn next(&mut self) -> Option<Self::Item> {
        self.inner.next()
    }
}

pub struct FieldVisitor<'a, F> {
    event_format: &'a F,
    fields: &'a mut FieldRecorder,
}

impl<'a, F: FormatEvent> FieldVisitor<'a, F> {
    fn new(event_format: &'a F, fields: &'a mut FieldRecorder) -> Self {
        Self {
            event_format,
            fields,
        }
    }

    fn record_field(&mut self, field: &Field, value: RecordedValue) {
        if self.event_format.accept_field(field, &value) {
            self.fields.push(field, value);
        }
    }
}

impl<'a, F: FormatEvent> Visit for FieldVisitor<'a, F> {
    fn record_f64(&mut self, field: &Field, value: f64) {
        self.record_field(field, value.into());
    }

    fn record_i64(&mut self, field: &Field, value: i64) {
        self.record_field(field, value.into());
    }

    fn record_u64(&mut self, field: &Field, value: u64) {
        self.record_field(field, value.into());
    }

    fn record_bool(&mut self, field: &Field, value: bool) {
        self.record_field(field, value.into());
    }

    fn record_str(&mut self, field: &Field, value: &str) {
        self.record_field(field, value.into());
    }

    fn record_error(&mut self, field: &Field, value: &(dyn std::error::Error + 'static)) {
        self.record_field(field, format!("{}", value).into());
    }

    fn record_debug(&mut self, field: &Field, value: &dyn std::fmt::Debug) {
        self.record_field(field, format!("{:#?}", value).into());
    }
}


#[derive(Clone, Debug)]
pub enum RecordedValue {
    F64(f64),
    I64(i64),
    U64(u64),
    Bool(bool),
    String(String),
}

impl Serialize for RecordedValue {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
        where
            S: Serializer,
    {
        match self {
            RecordedValue::F64(v) => serializer.serialize_f64(*v),
            RecordedValue::I64(v) => serializer.serialize_i64(*v),
            RecordedValue::U64(v) => serializer.serialize_u64(*v),
            RecordedValue::Bool(v) => serializer.serialize_bool(*v),
            RecordedValue::String(v) => serializer.serialize_str(v),
        }
    }
}

impl From<f64> for RecordedValue {
    fn from(v: f64) -> Self {
        Self::F64(v)
    }
}

impl From<i64> for RecordedValue {
    fn from(v: i64) -> Self {
        Self::I64(v)
    }
}

impl From<u64> for RecordedValue {
    fn from(v: u64) -> Self {
        Self::U64(v)
    }
}

impl From<bool> for RecordedValue {
    fn from(v: bool) -> Self {
        Self::Bool(v)
    }
}

impl From<String> for RecordedValue {
    fn from(v: String) -> Self {
        Self::String(v)
    }
}

impl From<&str> for RecordedValue {
    fn from(v: &str) -> Self {
        Self::String(v.to_owned())
    }
}
