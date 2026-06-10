# models/

Model artifacts are **never committed**. `MANIFEST.toml` is the single source of truth:
the image build script fetches each artifact over HTTPS, verifies the pinned SHA256,
and bakes it into the `meridiand` image (SPEC §9.6). The deployed device never
downloads models at runtime.
