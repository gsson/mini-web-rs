pub mod format;
pub mod logstash;

use crate::json_logging::format::FormatEvent;
use crate::json_logging::logstash::LogstashFormat;
use serde::{Serialize, Serializer};
use std::io::Write;
use std::marker::PhantomData;
use std::{fmt, io};
use tracing::field::{Field, Visit};
use tracing::span::{Attributes, Record};
use tracing::{Event, Id, Level, Subscriber};
use tracing_subscriber::fmt::MakeWriter;
use tracing_subscriber::layer::Context;
use tracing_subscriber::registry::LookupSpan;

#[derive(Default)]
pub(crate) struct RecordedFields {
    fields: Vec<(&'static str, RecordedValue)>,
}

impl RecordedFields {
    pub fn from_event(event_format: &impl FormatEvent, event: &Event) -> RecordedFields {
        let mut fields = RecordedFields::default();
        event.record(&mut FieldVisitor::new(event_format, &mut fields));
        fields
    }

    pub fn from_attributes(
        event_format: &impl FormatEvent,
        attributes: &Attributes,
    ) -> RecordedFields {
        let mut fields = RecordedFields::default();
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

impl<'a> IntoIterator for &'a RecordedFields {
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

pub struct Layer<S, E = LogstashFormat, W = fn() -> io::Stdout> {
    record_separator: Vec<u8>,
    make_writer: W,
    event_format: E,
    _inner: PhantomData<S>,
}

impl<S> Default for Layer<S> {
    fn default() -> Self {
        Self {
            record_separator: vec![b'\n'],
            make_writer: io::stdout,
            event_format: Default::default(),
            _inner: Default::default(),
        }
    }
}

impl<S, E, W> Layer<S, E, W>
where
    E: format::FormatEvent + 'static,
    S: Subscriber + for<'a> LookupSpan<'a>,
    W: for<'writer> MakeWriter<'writer> + 'static,
{
    pub fn record_separator(self, separator: impl Into<Vec<u8>>) -> Layer<S, E, W> {
        Layer {
            record_separator: separator.into(),
            ..self
        }
    }

    pub fn event_format<E2>(self, event_format: E2) -> Layer<S, E2, W>
    where
        E2: format::FormatEvent + 'static,
    {
        Layer {
            event_format,
            record_separator: self.record_separator,
            make_writer: self.make_writer,
            _inner: self._inner,
        }
    }

    pub fn with_writer<W2>(self, make_writer: W2) -> Layer<S, E, W2>
    where
        W2: for<'writer> MakeWriter<'writer> + 'static,
    {
        Layer {
            make_writer,
            event_format: self.event_format,
            record_separator: self.record_separator,
            _inner: self._inner,
        }
    }

    fn write_event(&self, event: &Event<'_>, ctx: Context<'_, S>) {
        let mut serializer = serde_json::Serializer::new(self.make_writer.make_writer());
        self.event_format
            .format_event(&mut serializer, event, ctx)
            .unwrap();
        let mut inner = serializer.into_inner();
        inner.write_all(&self.record_separator).unwrap();
    }
}

pub struct FieldVisitor<'a, F> {
    event_format: &'a F,
    fields: &'a mut RecordedFields,
}

impl<'a, F: format::FormatEvent> FieldVisitor<'a, F> {
    fn new(event_format: &'a F, fields: &'a mut RecordedFields) -> Self {
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

#[derive(Copy, Clone)]
pub enum DisplayLevelFilter {
    Off,
    All,
    Level(Level),
    Event,
}

impl DisplayLevelFilter {
    pub const ERROR: DisplayLevelFilter = Self::from_level(Level::ERROR);
    pub const WARN: DisplayLevelFilter = Self::from_level(Level::WARN);
    pub const INFO: DisplayLevelFilter = Self::from_level(Level::INFO);
    pub const DEBUG: DisplayLevelFilter = Self::from_level(Level::DEBUG);
    pub const TRACE: DisplayLevelFilter = Self::from_level(Level::TRACE);

    #[inline]
    const fn from_level(level: Level) -> DisplayLevelFilter {
        DisplayLevelFilter::Level(level)
    }

    #[inline]
    pub fn is_enabled(&self, event: &Event, span_level: &Level) -> bool {
        let filter_level = match self {
            DisplayLevelFilter::Level(level) => level,
            DisplayLevelFilter::Event => event.metadata().level(),
            DisplayLevelFilter::All => return true,
            DisplayLevelFilter::Off => return false,
        };
        filter_level >= span_level
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

impl<'a, F: format::FormatEvent> Visit for FieldVisitor<'a, F> {
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

    fn record_debug(&mut self, field: &Field, value: &dyn fmt::Debug) {
        self.record_field(field, format!("{:#?}", value).into());
    }
}

impl<S, E, W> tracing_subscriber::Layer<S> for Layer<S, E, W>
where
    S: Subscriber + for<'a> LookupSpan<'a>,
    E: format::FormatEvent + 'static,
    W: for<'writer> MakeWriter<'writer> + 'static,
{
    fn on_new_span(&self, attrs: &Attributes<'_>, id: &Id, ctx: Context<'_, S>) {
        let span = ctx.span(id).expect("Span not found, this is a bug");

        let mut extensions = span.extensions_mut();

        if extensions.get_mut::<RecordedFields>().is_none() {
            extensions.insert(RecordedFields::from_attributes(&self.event_format, attrs));
        }
    }

    fn on_record(&self, id: &Id, record: &Record<'_>, ctx: Context<'_, S>) {
        let span = ctx.span(id).expect("Span not found, this is a bug");
        let mut extensions = span.extensions_mut();

        if let Some(fields) = extensions.get_mut::<RecordedFields>() {
            fields.append_record(&self.event_format, record);
        }
    }

    fn on_event(&self, event: &Event<'_>, ctx: Context<'_, S>) {
        self.write_event(event, ctx);
    }
}
