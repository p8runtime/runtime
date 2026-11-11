# The p8 image: the `p8` command and nothing else. Build it from the
# repository's root:
#
#   docker build -f docker/Dockerfile -t p8 .
#
# A project's own Dockerfile starts `FROM p8` (see docker/README.md).

# Stage 1: p8 built from this checkout. Its dependency graph is Rust only
# (SECURITY.md section 2; preg_* is crates/p8-pcre, not the C PCRE2), so the
# Rust image is all it needs. `--build-arg JOBS=<n>` caps cargo's jobs.
FROM rust:1.96.0-trixie@sha256:58fe97504a0e4cbba5d85599619a589923d3e779472a6fb0840d58d1c4ba99d7 AS build
ARG JOBS=
WORKDIR /src
COPY . .
RUN cargo build --release --locked -p php-cli --bin p8 ${JOBS:+--jobs $JOBS} \
    && strip -o /p8 target/release/p8

# tini, statically linked, to run as PID 1. The kernel delivers no signal to
# a PID 1 that has not installed a handler for it, and p8 installs none, so
# without it `docker stop` and Ctrl-C would wait 10 s for SIGKILL. tini passes
# them on to p8, which stops at once.
FROM rust:1.96.0-trixie@sha256:58fe97504a0e4cbba5d85599619a589923d3e779472a6fb0840d58d1c4ba99d7 AS init
RUN apt-get update && apt-get install -y --no-install-recommends tini

# Stage 2: the binary on distroless/cc: glibc, libgcc_s, CA certificates and
# time zones, and no shell. The binary links glibc and libgcc_s dynamically
# (libgcc_s unwinds the panic that stops a request), so `scratch` and
# distroless/static (no libc) cannot run it, nor can distroless/base (no
# libgcc_s); cc is the smallest that can. `nonroot` is uid 65532.
FROM gcr.io/distroless/cc-debian13:nonroot@sha256:cc66d222f0a7ec996b78a99fe76a969bc80b539a5016ad866c14d1949bc6d0ab
COPY --from=init /usr/bin/tini-static /usr/local/bin/tini
COPY --from=build /p8 /usr/local/bin/p8
USER nonroot
EXPOSE 8000
ENTRYPOINT ["tini", "--", "p8"]
CMD ["--version"]
