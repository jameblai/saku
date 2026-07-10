# ADR 0004: Vision on message attachments and read; resize before send

## Status

Accepted

## Context

Saku needs vision. Pi attaches images on `prompt()` and can return image content from `read`. Auto-including older Discord thread images would surprise users and burn tokens. Large images stress a Raspberry Pi and Provider limits.

## Decision

- Include images only from the triggering Discord message attachments and from `read` of image files in the Workspace.
- Resize/compress images before sending them to the Provider.
- Do not scrape historical thread images into the prompt.

## Consequences

- Users must re-attach (or ask the agent to `read` a saved path) to discuss older images.
- Need a small image pipeline (decode, downscale, re-encode) in Rust.
