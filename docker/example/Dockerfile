FROM p8
COPY --chown=nonroot:nonroot . /app
RUN ["p8", "compile", "/app"]
CMD ["serve", "/app", "--listen", "0.0.0.0:8000"]
