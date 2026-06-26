# How is my data protected?

> **Who this is for:** anyone who wants to understand — in plain English — how
> ldgr keeps their financial life private. **No technical background needed.**
> If you'd like the deeper, more technical version, see
> [Want more detail?](#want-more-detail) at the end.

ldgr is built around one simple promise:

> **Only you can read your data. Not us, not your phone maker, not the cloud —
> nobody but you.**

This page explains how that works, using a picture you already know: a locked
box.

---

## The locked box

Imagine a strong, fireproof safe.

- **The safe is your vault** — it holds everything: your accounts, transactions,
  budgets, and balances.
- **Your password is the combination** — the only thing that opens the safe.
  You memorize it; you never write it into ldgr's servers, and it never leaves
  your device.
- **Your recovery key is a spare key, kept in a bank** — a backup way in, for
  the day you forget the combination. You store it somewhere very safe and only
  reach for it in an emergency.

Everything you put into ldgr goes **straight into the safe and is locked**
before it is ever stored or sent anywhere. Once it's locked, it looks like a
sealed, featureless box — meaningless to anyone who doesn't have the way in.

![A diagram showing you and your password unlocking your vault, which locks your
data into sealed boxes before it syncs to the cloud, where only locked boxes are
ever visible.](./assets/vault-overview-flow.svg)

*You unlock your vault with your password. Everything you save is locked inside
before it leaves your device. When it syncs, the cloud only ever sees sealed,
unreadable boxes.*

---

## What happens when you create a vault

When you set up ldgr for the first time, three things happen — all on your own
device:

1. **You choose a password.** This becomes the combination to your safe. ldgr
   never stores the password itself, anywhere.
2. **ldgr makes you a recovery key.** This is a long, randomly generated spare
   key, shown to you once as an "emergency kit" to write down or print. It's
   your backup way in if you ever forget your password.
3. **Your safe is ready.** From this moment on, everything you record — every
   account, every transaction — is locked inside the vault before it's saved.

You never have to think about locking and unlocking. It happens automatically
every time you open and close the app.

---

## What the server sees during sync

If you use ldgr on more than one device — say your laptop and your phone — your
data needs to travel between them. ldgr can sync it through the cloud so your
devices stay in step.

Here's the important part:

> **The cloud only ever sees sealed, locked boxes.** It never sees your
> accounts, your balances, or a single transaction.

This is what people mean by **"zero-knowledge"**: the service storing your data
has *zero knowledge* of what's inside it. The locking and unlocking only ever
happens on your own devices, using your password — which the server never has.

That's true even for us. **ldgr — the company and the servers — cannot read
your data.** There's no master key, no back door, and no "reset my data" button
on our side, because there's nothing on our side that could open your safe.

---

## Your recovery key

Because *only you* can open your vault, there's an important trade-off to
understand.

**What it is:** a one-time backup key, generated just for you when you create
your vault. Think of it as the spare key to your safe.

**Why it matters:** your password lives only in your memory. If you ever forget
it, the recovery key is the *only* other way to get back into your vault. With
it, you can unlock your data and set a new password.

**How to store it safely:**

- Write it down or print the emergency kit ldgr gives you.
- Keep it somewhere secure and offline — a real safe, a locked drawer, or a
  bank safe-deposit box.
- **Don't** store it as a screenshot in your photos, or in a plain note synced
  to the cloud. That's like taping the spare key to the front of the safe.
- Consider keeping a second copy in another trusted location.

**The honest truth you need to know:**

> If you lose **both** your password **and** your recovery key, your data
> **cannot be recovered** — by you, by us, or by anyone. That's the flip side of
> nobody else being able to read it.

This isn't a limitation we can "fix" — it's the very thing that keeps your data
private. We can't have it both ways: a back door for you would be a back door
for everyone.

---

## Frequently asked questions

### Can anyone read my data?

No. Your data is locked the moment you save it, and the only thing that can
unlock it is your password (or your recovery key). We don't have either one, so
we can't read your data — and neither can anyone who steals it from a server or
intercepts it on the network. They'd only get sealed, unreadable boxes.

### What if I forget my password?

Use your **recovery key** — the spare key you saved when you set up your vault.
It lets you back in so you can choose a new password. This is exactly what the
recovery key is for, so keep it somewhere safe. If you've forgotten your
password *and* lost your recovery key, unfortunately your data can't be
recovered, because there's no other way in.

### Is my data safe in the cloud?

Yes. Your data is locked **on your device** before it ever travels to the cloud,
so the cloud only ever stores sealed boxes it can't open. Even if the cloud
provider were hacked, an attacker would walk away with locked boxes and no way
to open them.

### What if ldgr shuts down?

Your data isn't trapped with us. ldgr is **local-first**, which means the real
copy of your data lives on your own device — not only on our servers. You can
keep using the app, and you can export your records to a standard, open
bookkeeping format that other tools can read. If ldgr ever disappeared
tomorrow, your financial history would still be yours.

---

## Want more detail?

This page is the friendly overview. If you'd like to go deeper:

- **[How the vault container works](./vault-format-guide.md)** — an
  intermediate guide for readers with some technical background. It explains the
  vault's structure, how keys are organized, and how password changes and
  recovery actually work, without requiring a cryptography degree.
- **[ldgr Architecture](../ldgr-architecture.md)** — the full system design,
  including the encryption architecture and security decisions, for engineers
  and auditors.
