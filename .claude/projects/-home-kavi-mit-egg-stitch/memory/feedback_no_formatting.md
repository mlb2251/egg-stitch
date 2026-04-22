---
name: No nuisance formatting
description: Do not reformat code that isn't being changed — only touch lines relevant to the task
type: feedback
---

Don't make unrelated formatting changes to code I'm not otherwise modifying.

**Why:** User finds gratuitous reformatting noisy and distracting from the actual changes.

**How to apply:** When editing files, only change the lines needed for the task. Don't reformat surrounding code, even if cargo fmt would change it. Let the user run cargo fmt themselves if they want.
