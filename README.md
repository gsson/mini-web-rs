## Introduction

Minimal Rust axum based web server setup with Prometheus metrics and logging

## To run

```
$ cargo run
```

It will listen for requests on port 3000.

Prometheus metrics on `/prometheus`

Important high value business logic on `/api/hello/:name`