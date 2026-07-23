# Subscribe, Snapshot, Stream: I Built a File Sync Engine Like It Was Market Data

*A bidirectional Proton Drive sync in Rust, an AI that wrote 1,900 lines, and why trusting those lines was the actual engineering.*

Before I write sync code, I think about order books.

When you sync live market data, you never poll for the whole world on a loop. You subscribe to the feed first, so nothing slips past you while you set up. Then you pull one full snapshot to know the truth right now. Then you stop asking for everything and just apply the deltas as they stream in. Subscribe, snapshot, stream. It is the only sane way to stay in sync with something that keeps changing under you.

I wanted the same thing for files: a bidirectional sync engine for Proton Drive, in Rust, between a local folder and the cloud. And the naive design is exactly the thing you would never do with a price feed. Every cycle, walk the entire remote folder tree, top to bottom, and diff it. That is O(folders). Ten thousand folders, one changed file, and you still pay for ten thousand. You are re-reading a whole library to find out someone moved one book. Slow, and honestly a little dumb.

## The optimization that died on my real account

So I asked the obvious question. If I change one file buried five folders deep, does the parent folder notice? Does anything up the tree flag that something inside it changed? If so, I could skip every untouched subtree and only descend where the dirt is.

I did not reason about this from docs. I tested it live, against my real Proton account. Changed a deep file, watched the parents. The answer was a flat no. There is no "something changed here" marker anywhere in the tree. Folders do not even track their own rename time. My clever optimization was dead on arrival. That stung for about an hour. Then it got interesting.

Because poking at a live API is never wasted. While I was in there, I found the thing I actually needed. Proton keeps a volume event stream: a running changelog of everything that happens to your files, that you page through with a cursor. It is what the official app uses under the hood, and it is not exposed as a command. It is right there in the logs, a `GET` against `.../drive/v2/volumes/{volumeId}/events/{cursor}`, polled every thirty seconds. It is O(changes), not O(folders). It was the streaming feed I had been describing to myself the whole time, without knowing it already existed.

## Failures I earned

Before I found the event stream, my first instinct for speed was the lazy one: keep the full walk, just parallelize it. Fan out, hit many folders at once. It blew up. The Proton CLI leans on a shared SQLite cache, and running it concurrently throws `SQLITE_BUSY` and dies, dumping a JavaScript stack trace straight into the output I was trying to parse as JSON. I isolated the cache per invocation. Still failed, because there is a second shared session store you cannot isolate without breaking auth. Green tests on my laptop, corpse in the real world. I killed that branch. The lesson stuck harder than the code: verify against the real thing, not the fake that agrees with you.

Auth was the wall I expected to lose weeks to. To read that event stream you need a valid session, and I did not want to reimplement Proton's login. Here is the part I did not know going in, and had to confirm by reading the SDK source: detecting changes needs only a session, not the decryption keys. Detection is metadata. So I reused the logged-in CLI's session straight out of the OS keyring. I wanted a fully independent login I owned end to end, but the CLI can only consume a browser login, not mint one headlessly, so that door was closed for now. Pragmatic call: reuse the session today, do it properly later, and write the decision down honestly instead of pretending it was the plan all along.

Then the subtle one. The trap the whole design balanced on.

An event from the stream hands you a raw node id, something like `39NX…`. But my local index, and the directory listings I diff against, key everything by a composed id: volume and node stitched together, `v45s…~39NX…`. Two different names for the same file. If you do not bridge them, nothing throws. No error, no crash. The system just quietly decides it recognizes none of the changes and falls back to walking the whole tree, forever, silently defeating the entire point. A one-line mismatch that fails without a single error in the log. That was the linchpin.

## 1,900 lines I refused to trust

Here is where the AI part comes in, and where it gets interesting. A cloud agent implemented the entire event-driven engine on its own branch while I worked on other things. Roughly 1,900 lines. I read it top to bottom and it looked genuinely good. Clean seams, the HTTP transport injected, no networking bolted into the core.

And that was the exact moment to be most careful. 1,900 plausible lines are not 1,900 correct lines. Writing them was the easy part. Trusting them was the whole job.

So I built the gate the design actually needed. One live test, on my real account, proving that the raw id a file's create event carries really does bridge to the composed id stored in my index, for the same file. I uploaded a probe file, watched its create event land in the stream, and checked the bridge. It held. If that single test had failed, the design was invalid and nothing else mattered. Everything downstream was resting on that one equality.

## The bug the tests were happy to ignore

Then I had a stronger advisor model tear into it adversarially, and it caught a bug my tests slept right through.

Picture the daemon switched off. While it is down, I edit a local file. Then I start it back up. A fresh process has no memory of what happened while it was dead. My design would subscribe to the event stream from the current cursor, start applying remote changes, and silently never notice the local edit I made during the outage. For up to about ten minutes, until the next full scan happened to catch it, that file just would not sync. No error. No log line. Just quietly wrong.

The tests were green before this fix. They were testing the wrong world. Every one of them started from a running, consistent state; the failure only exists across a restart, in the gap between two processes. And the fix is the market-data pattern, exactly. On startup you do not stream first. You take the full snapshot to establish the truth, and only then switch to the delta feed. Subscribe, snapshot, stream, in that order, or you drop the ticks that happened while you were asleep. I had known this cold for price feeds. I still almost shipped the file version without it.

## The payoff, and the papercuts

The rest was the usual honest grind. Eventual consistency bit me: I would trash a test file, and the listing kept cheerfully showing it for a few seconds, poisoning the next run. A Unix socket refused to bind because my temp path was buried so deep the path was literally too long for the OS limit. Small papercuts, each one its own little detour.

I filed real follow-up issues for the limitations I found instead of pretending it was flawless, and I shipped the whole feature off by default. Opt in, or it cannot touch you.

Then the payoff I actually wanted: I made a real change on the server, and watched the daemon pick it up from the event stream alone. Zero full folder walks. The feed, doing its job.

## What the collaboration actually was

I keep coming back to the ratio of it. The AI wrote the bulk, about 1,900 lines, on its own. A stronger model caught the bug I would have shipped. And yet the engineering, the part that was genuinely hard and genuinely mine, was none of the writing. It was refusing to merge on faith. It was building the one live test that could prove the linchpin true or false, and being willing to throw the whole thing out if it came back false.

AI did not replace the craft. It amplified it, and only because I would not trust it blindly. The cloud agent wrote the bulk, the advisor model caught the bug, and I made the calls. That is the collaboration.

We are craftsmen. We deliver robust, quality work, and we do it right and cleanly. The typing got faster. The discipline did not get optional.
