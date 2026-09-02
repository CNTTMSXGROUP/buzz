# Composer F1: retained failure evidence

This is a publication of existing evidence, not a new test run or a fix.

Cloud candidate: `d280d36c1f5b07322fedba52bf0b8f766e192cd9`.
Pinned comparison base: `0e878664b08cdf7fb2d89d940bc2aa92cdc485f7`.

## Outcomes (distinct executions)

- Original full candidate mentions invocation: **78 passed / 2 failed out of 80**, one worker, zero retries. Its output includes `team-mentions.spec.ts`; do not equate it with a later 79-case full-file review run.
- Isolated exact-base pair: **2/2 pass**; an additional successful-trace capture pair: **2/2 pass**.
- Isolated unchanged candidate pair: **2/2 pass**, with matched fixtures/browser settings and zero retries.
- [Exact-head reviewer report](https://github.com/block/buzz/pull/7129#pullrequestreview-5092843946): separate systems-lane `mentions.spec.ts` **79/79 pass**, one worker, zero retries; a different adjacent product/UI-lane failure passed 3/3 isolated. These are reviewer-reported results, not a replacement of the original 80-test invocation.

## Relevant original trace excerpts

The excerpts below retain the literal incremental paragraph fragments extracted in the prior trace comparison; they are not a reconstructed full DOM. Only relevant mock-UI fragments and call IDs are shared. Local paths, configuration, network payloads and transcripts are excluded.

### Person: `mentions.spec.ts:3004`, failed assertion `:3024`

Test: **sent non-member person mention uses the normal mention style**.

Original trace SHA-256: `269b254a01047dfb6108efb1593bdd8da4b6fc39975637c3761cc1a9ab37dd5f`.

```text
call@3093 press Enter:
["P", {}, "Loop in ",
 ["SPAN", {"spellcheck":"false","class":"mention-prefix-hidden"}, "@"],
 ["SPAN", {"spellcheck":"false","class":"mention-chip inline-chip-with-icon inline-chip-icon-human human-mention-highlight"}, "outsider"],
 " "]

call@3095 keyboardType text=" please":
["P", {}, "Loop in @outsiderplease"]

call@3097 click send-message
call@3099 expect visible sent [data-mention] with text outsider: fails
```

### Managed agent: `mentions.spec.ts:3028`, failed assertion `:3057`

Test: **sent managed non-member agent mention uses the agent mention style**.

Original trace SHA-256: `3a2898966b356b30fd07dbf87da12198acc31721936bca43cb07d9e2cb64559f`.

```text
call@34 press Enter:
["P", {}, "Loop in ",
 ["SPAN", {"spellcheck":"false","class":"mention-prefix-hidden"}, "@"],
 ["SPAN", {"spellcheck":"false","class":"mention-chip inline-chip-with-icon inline-chip-icon-agent agent-mention-highlight"}, "charlie"],
 " "]

call@36 keyboardType text=" too":
["P", {}, "Loop in @charlietoo"]

call@38 click send-message
call@40 expect visible sent [data-mention] with text charlie: fails
```

Both are in the mock `bob-tyler` DM. The chip and literal separator exist after Enter, and are lost during suffix typing **before Send**. The attached original failure screenshots show the resulting **sent** plain text, not the pre-send state; the trace excerpts establish the earlier boundary.

## Limits and preservation

Full original failed traces, screenshots, DOM snapshots and logs remain preserved by the reporting workspace. Raw trace archives are not publicly uploaded because they include local execution paths/configuration and broader captured resources. The excerpts above are the shareable relevant trace evidence; raw traces remain available for a separately authorized investigation after appropriate disclosure review.

Root cause is unknown. Governing insertion/caret source was byte-identical in the recorded base/candidate comparison. There is no proven exact-base failure, candidate-only/cloud regression, or demonstrated fix in #7128's multiword/full-name extension. Timing/suite-order sensitivity is possible, not established. Isolated passes do not erase the failure, justify a flakiness dismissal, or establish resolution.

The [review](https://github.com/block/buzz/pull/7129#pullrequestreview-5092843946) treats F1 as nonblocking **for cloud PR #7129**, with separate mentions/composer follow-up. This does not waive human security review, authorize merge, or certify native/live-cloud behavior. See also the [original public evidence summary](https://github.com/block/buzz/pull/7129#issuecomment-5513218962); its publication-era cloud hold is superseded only by the later review's cloud-specific disposition.
