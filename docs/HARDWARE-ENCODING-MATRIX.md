# Hardware encoding verification matrix

No physical Windows GPU/driver combination has been verified for these encoders.
A listed encoder being present in ffmpeg does not prove its driver or device can
successfully encode. An explicitly selected hardware encoder fails visibly if it
cannot run; it never silently falls back to software.

| Vendor | Encoder | GPU/Driver tested | Result | Fallback behaviour |
| --- | --- | --- | --- | --- |
| NVIDIA | NVENC H.264 / HEVC | None | Unverified | Error; no software fallback |
| Intel | QSV H.264 / HEVC | None | Unverified | Error; no software fallback |
| AMD | AMF H.264 / HEVC | None | Unverified | Error; no software fallback |

For a useful report include Windows version and build, ytTrayDL version,
ffmpeg version/build and encoder, GPU model and driver, profile, input and
output formats, steps, outcome and a sanitized error if applicable. Do not
include personal media or paths without reviewing them first.
