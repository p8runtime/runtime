# p8 in Docker

A PHP project in a container, served by p8: build the p8 image once, then
give the project a four-line Dockerfile.

## 1. Build the p8 image

From the root of this repository:

```
docker build -f docker/Dockerfile -t p8 .
```

The image holds the `p8` binary only, on distroless (glibc, CA certificates
and time zones, no shell), and runs as the user `nonroot`. `docker run --rm p8`
prints its version. Add `--build-arg JOBS=2` to cap cargo's jobs on a small
machine.

## 2. The project's Dockerfile

Put this beside the project's files (`docker/example/` is a working copy):

```
FROM p8
COPY --chown=nonroot:nonroot . /app
RUN ["p8", "compile", "/app"]
CMD ["serve", "/app", "--listen", "0.0.0.0:8000"]
```

Then, in the project's folder:

```
docker build -t my-site .
docker run --rm -p 8000:8000 my-site
```

and open http://localhost:8000/. Add a `.dockerignore` with `.p8` in it, so a
package written on your own machine is not copied in.

- The `RUN` line is in exec form because the image has no shell.
- `--chown` makes the files the server's own, so `p8 compile` can write
  `/app/.p8/package` and the application can write its uploads.
- `--listen 0.0.0.0:8000` is needed: `p8 serve` listens on `127.0.0.1:8000`
  by default, which nothing outside the container can reach.
- To stop it, press Ctrl-C or run `docker stop`.

## 3. What `p8 compile` in the image buys

`p8 compile /app` compiles every PHP file once, when the image is built, into
`/app/.p8/package`. `p8 serve` takes each file from the package, so a request
compiles nothing. Each request's log line says so:

```
[Wed Nov 11 13:30:01 2026] 172.17.0.1:40512 [200]: GET / - 0 files compiled from source
```

The package records each file by its full path, so compile it inside the
image, at the path it is served from. A package written for another path, by
another build of p8, or with `p8 compile --cli` is not used: the server says
so once on startup and compiles every file from source.

## 4. When a file changes in a running container

`p8 serve` compares each file with the package before using it. An edited
or new file is compiled from source at the next request and runs its new
code (the log line says `1 file compiled from source`); the other files still
come from the package. The package itself is not rewritten, and the edit is
gone when the container is replaced: rebuild the image to keep it.

## WordPress

`docker/wordpress/` builds WordPress 7.1.4 (the release the WordPress gate
uses) on the p8 image. The database is MariaDB in a second container, as in
the gate:

```
docker build -t p8-wordpress docker/wordpress
docker network create wp
password=$(od -An -N16 -tx1 /dev/urandom | tr -d ' \n')
docker run -d --name wp-db --network wp --network-alias db \
  -e MARIADB_DATABASE=wordpress -e MARIADB_USER=wordpress \
  -e MARIADB_PASSWORD=$password -e MARIADB_RANDOM_ROOT_PASSWORD=1 mariadb:11.4
docker run -d --name wp --network wp -e WORDPRESS_DB_PASSWORD=$password \
  -p 8000:8000 p8-wordpress
```

Open http://localhost:8000/ and WordPress's installer runs. The database
password is made up when you run these commands; `wp-config.php` reads it
from `WORDPRESS_DB_PASSWORD` (WordPress's own setting, not p8's). Posts and
users live in the `wp-db` container; uploads live in the `wp` container
until it is removed.

## Testing the flow

```
docker/test.sh [-j <cargo jobs>] [--no-wordpress]
```

builds the p8 image, the example and WordPress, runs them, requests their
pages and the example's stylesheet, checks the bodies and that the log says
`0 files compiled from source`, edits a file in the running example, and
removes everything it started. It exits non-zero on any failure. It needs
Docker, so no cargo gate runs it.
