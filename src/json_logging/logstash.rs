use crate::json_logging::format::{
    write_extension_fields, DefaultSpanFormat, FormatEvent, FormatSpan, SerializableSpanList,
};
use crate::json_logging::{DisplayLevelFilter, FieldRecorder};
use serde::ser::SerializeMap as _;
use serde::Serializer;
use std::collections::HashSet;
use std::fmt::Write as _;
use tracing::{Event, Level, Metadata, Subscriber};
use tracing_subscriber::layer::Context;
use tracing_subscriber::registry::LookupSpan;

pub struct LogstashFormat<SF = DefaultSpanFormat> {
    display_version: bool,
    display_timestamp: bool,
    display_logger_name: bool,
    display_thread_name: bool,
    display_level: bool,
    display_level_value: bool,
    display_span_list: Option<DisplayLevelFilter>,
    display_stack_trace: Option<DisplayLevelFilter>,
    span_format: SF,
}

const fn level_value(level: &Level) -> u64 {
    match *level {
        Level::ERROR => 3,
        Level::WARN => 4,
        Level::INFO => 5,
        Level::TRACE => 6,
        Level::DEBUG => 7,
    }
}

impl<SF> LogstashFormat<SF> {
    pub fn with_timestamp(self, display_timestamp: bool) -> Self {
        Self {
            display_timestamp,
            ..self
        }
    }
    pub fn with_version(self, display_version: bool) -> Self {
        Self {
            display_version,
            ..self
        }
    }
    pub fn with_logger_name(self, display_logger_name: bool) -> Self {
        Self {
            display_logger_name,
            ..self
        }
    }
    pub fn with_thread_name(self, display_thread_name: bool) -> Self {
        Self {
            display_thread_name,
            ..self
        }
    }
    pub fn with_level(self, display_level: bool) -> Self {
        Self {
            display_level,
            ..self
        }
    }
    pub fn with_level_value(self, display_level_value: bool) -> Self {
        Self {
            display_level_value,
            ..self
        }
    }
    pub fn with_span_list(self, display_span_list: Option<DisplayLevelFilter>) -> Self {
        Self {
            display_span_list,
            ..self
        }
    }
    pub fn with_stack_trace(self, display_stack_trace: Option<DisplayLevelFilter>) -> Self {
        Self {
            display_stack_trace,
            ..self
        }
    }
    pub fn span_format<FS2>(self, span_format: FS2) -> LogstashFormat<FS2> {
        LogstashFormat {
            display_version: self.display_version,
            display_timestamp: self.display_timestamp,
            display_logger_name: self.display_logger_name,
            display_thread_name: self.display_thread_name,
            display_level: self.display_level,
            display_stack_trace: self.display_stack_trace,
            display_level_value: self.display_level_value,
            display_span_list: self.display_span_list,
            span_format,
        }
    }
}

impl Default for LogstashFormat {
    fn default() -> Self {
        Self {
            display_version: true,
            display_timestamp: true,
            display_logger_name: true,
            display_thread_name: true,
            display_level: true,
            display_level_value: true,
            display_stack_trace: None,
            display_span_list: None,
            span_format: Default::default(),
        }
    }
}

fn format_stack_trace<SS>(
    event: &Event<'_>,
    ctx: &Context<'_, SS>,
    filter: DisplayLevelFilter,
) -> String
where
    SS: Subscriber + for<'a> LookupSpan<'a>,
{
    fn append_line(stack_trace: &mut String, metadata: &Metadata<'_>) {
        writeln!(
            stack_trace,
            "  at {}({}:{})",
            metadata.target(),
            metadata.file().unwrap_or("<unknown>"),
            metadata.line().unwrap_or(0)
        )
        .unwrap();
    }
    let mut stack_trace = String::new();
    if let Some(scope) = ctx.event_scope(event) {
        for span in scope.from_root() {
            let span_metadata = span.metadata();
            if filter.is_enabled(event, span_metadata.level()) {
                append_line(&mut stack_trace, span_metadata);
            }
        }
    }
    let event_metadata = event.metadata();
    if filter.is_enabled(event, event_metadata.level()) {
        append_line(&mut stack_trace, event_metadata);
    }
    if !stack_trace.is_empty() {
        stack_trace.truncate(stack_trace.len() - 1);
    }
    stack_trace
}

const RESERVED_NAMES: [&str; 8] = [
    "@version",
    "@timestamp",
    "thread_name",
    "logger_name",
    "level",
    "level_value",
    "stack_trace",
    "spans",
];

impl<FS> FormatEvent for LogstashFormat<FS>
where
    FS: FormatSpan,
{
    fn format_event<S: Serializer, SS: Subscriber + for<'a> LookupSpan<'a>>(
        &self,
        serializer: S,
        event: &Event<'_>,
        ctx: Context<'_, SS>,
    ) -> Result<S::Ok, S::Error> {
        let event_metadata = event.metadata();
        let event_level = event_metadata.level();

        let mut s = serializer.serialize_map(None)?;
        if self.display_version {
            s.serialize_entry("@version", "1")?;
        }

        if self.display_timestamp {
            s.serialize_entry("@timestamp", &chrono::Local::now())?;
        }

        if self.display_thread_name {
            let thread = std::thread::current();
            if let Some(name) = thread.name() {
                s.serialize_entry("thread_name", name)?;
            }
        }

        if self.display_logger_name {
            s.serialize_entry("logger_name", event_metadata.target())?;
        }

        if self.display_level {
            s.serialize_entry("level", event_level.as_str())?;
        }

        if self.display_level_value {
            s.serialize_entry("level_value", &level_value(event_level))?;
        }

        if let Some(filter) = self.display_stack_trace {
            s.serialize_entry("stack_trace", &format_stack_trace(event, &ctx, filter))?;
        }

        if let Some(filter) = self.display_span_list {
            s.serialize_entry(
                "spans",
                &SerializableSpanList(&self.span_format, event, &ctx, filter),
            )?;
        }

        let mut seen = HashSet::from(RESERVED_NAMES);

        let event_fields = FieldRecorder::from_event(self, event);

        write_extension_fields(&mut seen, &mut s, &event_fields)?;
        if let Some(scope) = ctx.event_scope(event) {
            for span in scope {
                if let Some(span_fields) = span.extensions().get::<FieldRecorder>() {
                    write_extension_fields(&mut seen, &mut s, span_fields)?;
                }
            }
        }
        s.end()
    }
}
