# Building the artifact image

`/artifact` in the image is the Docker build context, with all Rust crates in
`vendor/` and Python wheels in `wheels/`. To rebuild the image from it:

```bash
docker build -t estitch-artifact .
```

To regenerate the build context from an egg-stitch checkout:

```bash
artifact/prepare.sh
docker build -t estitch-artifact artifact/build
docker save estitch-artifact | gzip > estitch-artifact.tar.gz
```
