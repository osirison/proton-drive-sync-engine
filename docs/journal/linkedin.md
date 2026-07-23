**I'm building my own file-sync engine, and letting AI write most of it.**

The what and the why, minus the jargon.

I keep my important files on Proton Drive, but I live on Linux, and there's still no proper native sync client for it here. That effortless folder that just stays in sync with the cloud, both directions, the way Dropbox does everywhere else, simply doesn't exist for me. So two things lined up nicely: a real gap I feel, and the perfect excuse to finally learn Rust by building something I'd actually use.

The obvious approach is simple and wasteful: every few seconds, compare everything on my computer against everything in the cloud. It works, but it's like re-reading an entire library each time just to find the one book someone moved.

So the real challenge became this: detect only what actually changed, and sync just that.

Here's the part I find genuinely exciting. I didn't write most of the code. A cloud AI agent built the core of it while I worked on other things, and on a first read it looked great.

That was exactly the moment to slow down.

Plausible code is not the same as correct code. So the real work became mine again: verification. I tested it against my actual account instead of a mock. I had a second, stronger AI model review it adversarially, and it caught a real bug I would otherwise have shipped. I fixed it, wrote down honestly what was still rough, and released it switched off by default so it couldn't surprise anyone.

The lesson I keep coming back to:

AI made the typing fast. It didn't make the judgment optional.

Writing the code was the easy part. Deciding whether to trust it was the engineering.

#BuildInPublic #AI #Rust #SoftwareEngineering
