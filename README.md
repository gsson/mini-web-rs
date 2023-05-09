## Introduction

Minimal Rust Axum based web server setup with Prometheus metrics and Logstash style JSON logging

The goal of this project is to build the smallest service possible that could work as a drop-in replacement (from an operational point) for a Spring Boot based Java service running in a Kubernetes cluster.

## Features

Emits Prometheus metrics for request timings (several lines omitted for brevity);
```
# HELP http_server_requests_seconds Server request metrics
# TYPE http_server_requests_seconds histogram
http_server_requests_seconds_bucket{method="GET",status="200",uri="/api/hello/:name",le="0.5"} 3
http_server_requests_seconds_sum{method="GET",status="200",uri="/api/hello/:name"} 0.0006075659999999999
http_server_requests_seconds_count{method="GET",status="200",uri="/api/hello/:name"} 3
http_server_requests_seconds_bucket{method="GET",status="404",uri="",le="0.5"} 2
http_server_requests_seconds_sum{method="GET",status="404",uri=""} 0.000030874
http_server_requests_seconds_count{method="GET",status="404",uri=""} 2
...
```

And Logstash style JSON on stdout;
```
{"@version":"1","@timestamp":"2021-11-06T19:19:27.554384+00:00","thread_name":"tokio-runtime-worker","logger_name":"mini_web::observability","level":"INFO","level_value":5,"matched_path":"/api/hello/:name","requested_uri":"/api/hello/world","method":"GET","elapsed_time":0,"status":"200","message":"Request complete: 200 OK"}
{"@version":"1","@timestamp":"2021-11-06T19:19:32.139903+00:00","thread_name":"tokio-runtime-worker","logger_name":"mini_web::observability","level":"INFO","level_value":5,"matched_path":"/prometheus","requested_uri":"/prometheus","method":"GET","elapsed_time":0,"status":"200","message":"Request complete: 200 OK"}
{"@version":"1","@timestamp":"2021-11-06T19:19:40.650868+00:00","thread_name":"tokio-runtime-worker","logger_name":"mini_web::observability","level":"INFO","level_value":5,"requested_uri":"/metrics","method":"GET","elapsed_time":0,"status":"404","message":"Request complete: 404 Not Found"}
```


## todo!()

* Additional request information in the logs (specific request headers etc)
* Externalised configuration
* Dockerisation
* Kubernetes [Liveness, Readiness and Startup Probes](https://kubernetes.io/docs/tasks/configure-pod-container/configure-liveness-readiness-startup-probes/)
* REST client example (with client logging and metrics)
* Database access example (with logging and metrics)
* Redis access example (with logging and metrics)

## To run

```
$ cargo run
```

It will listen for requests on port 3000.

Prometheus metrics on `/prometheus`

Important high value business logic on `/api/hello/:name`