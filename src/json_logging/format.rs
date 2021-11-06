use crate::json_logging::{DisplayLevelFilter, RecordedFields};
use serde::ser::{SerializeMap, SerializeSeq};
use serde::{Serialize, Serializer};
use std::collections::HashSet;
use tracing::field::Field;
use tracing::{Event, Subscriber};
use tracing_subscriber::layer::Context;
use tracing_subscriber::registry::{LookupSpan, SpanRef};

pub trait FormatSpan {
    fn format_span<S, Span>(&self, serializer: S, span: &SpanRef<Span>) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
        Span: Subscriber + for<'lookup> LookupSpan<'lookup>;
}

pub trait FormatEvent {
    fn accept_field<T>(&self, _field: &Field, _value: &T) -> bool {
        true
    }

    fn format_event<S: Serializer, SS: Subscriber + for<'a> LookupSpan<'a>>(
        &self,
        serializer: S,
        event: &Event<'_>,
        ctx: Context<'_, SS>,
    ) -> Result<S::Ok, S::Error>;
}

#[derive(Default)]
pub struct DefaultSpanFormat {
    display_location: bool,
    display_fields: bool,
}

impl DefaultSpanFormat {
    pub fn with_location(self, display_location: bool) -> Self {
        Self {
            display_location,
            ..self
        }
    }
    pub fn with_fields(self, display_fields: bool) -> Self {
        Self {
            display_fields,
            ..self
        }
    }
}

const RESERVED_SPAN_FIELDS: [&str; 5] = ["name", "target", "level", "file", "line"];

impl FormatSpan for DefaultSpanFormat {
    fn format_span<S, Span>(&self, serializer: S, span: &SpanRef<Span>) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
        Span: Subscriber + for<'lookup> LookupSpan<'lookup>,
    {
        let mut s = serializer.serialize_map(None)?;
        let metadata = span.metadata();
        s.serialize_entry("name", span.name())?;
        s.serialize_entry("target", metadata.target())?;
        s.serialize_entry("level", metadata.level().as_str())?;
        if self.display_location {
            if let Some(file) = metadata.file() {
                s.serialize_entry("file", file)?;
            }
            if let Some(line) = metadata.line() {
                s.serialize_entry("line", &line)?;
            }
        }
        if self.display_fields {
            if let Some(fields) = span.extensions().get::<RecordedFields>() {
                write_extension_fields(&mut HashSet::from(RESERVED_SPAN_FIELDS), &mut s, fields)?;
            }
        }
        s.end()
    }
}

pub(crate) fn write_extension_fields<S: SerializeMap>(
    seen: &mut HashSet<&str>,
    serialize_map: &mut S,
    fields: &RecordedFields,
) -> Result<(), S::Error> {
    for field in fields {
        if seen.insert(field.0) {
            serialize_map.serialize_entry(field.0, &field.1)?;
        }
    }
    Ok(())
}

pub(crate) struct SerializableSpan<'fmt_span, 'span, FmtSpan, Span>(
    pub(crate) &'fmt_span FmtSpan,
    pub(crate) &'span SpanRef<'fmt_span, Span>,
)
where
    Span: for<'lookup> LookupSpan<'lookup>;

impl<'a, 'b, FmtSpan, Span> Serialize for SerializableSpan<'a, 'b, FmtSpan, Span>
where
    FmtSpan: FormatSpan,
    Span: Subscriber + for<'lookup> LookupSpan<'lookup>,
{
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        self.0.format_span(serializer, self.1)
    }
}

pub(crate) struct SerializableSpanList<'a, FS, Span>(
    pub(crate) &'a FS,
    pub(crate) &'a Event<'a>,
    pub(crate) &'a Context<'a, Span>,
    pub(crate) DisplayLevelFilter,
)
where
    Span: for<'lookup> LookupSpan<'lookup>;

impl<'a, FS, SS> Serialize for SerializableSpanList<'a, FS, SS>
where
    FS: FormatSpan,
    SS: Subscriber + for<'lookup> LookupSpan<'lookup>,
{
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let mut s = serializer.serialize_seq(None)?;
        if let Some(scope) = self.2.event_scope(self.1) {
            for span in scope {
                if self.3.is_enabled(self.1, span.metadata().level()) {
                    s.serialize_element(&SerializableSpan(self.0, &span))?;
                }
            }
        }
        s.end()
    }
}
