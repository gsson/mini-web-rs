## Introduction

Minimal Rust Axum based web server setup with Prometheus metrics and Logstash style JSON logging

The goal of this project is to build the smallest service possible that could work as a drop-in replacement for a Spring Boot based Java service.

The `json_logging` module is loosely modeled after [tracing_subscriber::fmt](https://docs.rs/tracing-subscriber/0.3.1/tracing_subscriber/fmt/index.html). It's currently a bit bloated, but should probably be extracted into its own crate should it turn out to be useful.

## To run

```
$ cargo run
```

It will listen for requests on port 3000.

Prometheus metrics on `/prometheus`

Important high value business logic on `/api/hello/:name`