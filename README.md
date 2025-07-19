# ⚡ Tachyon

> A web server so fast it breaks causality... and the spec.

---
## 🧪 Benchmark: Plaintext — or How I Learned to Stop Worrying and Love UB

> *"It’s not a bug, it’s a feature. Also, it’s on fire."*

| Framework              | RPS         | % of max | Comments                          |
|------------------------|-------------|----------|-----------------------------------|
| **tachyon-concept-ub** | 9,742,487   | 🏆 100%  | 🔥 Pure undefined behavior glory  |
| **faf**                | 7,025,809   | 72.1%    | Tried to follow the rules… lost   |
| **tachyon-concept**    | 6,020,793   | 61.8%    | Stable, safe, *boring*            |
| **mrhttp**             | 5,926,138   | 60.8%    | Python?? In *this* economy?       |

**`tachyon-concept-ub`** is what happens when you:
- Ignore memory safety.
- Abuse `unsafe`.
- Pre-chew syscalls before the kernel even knows it needs them.
- And write code your future self won’t recognize — because it’ll be corrupted in L1 cache.

---

### ☠️ Stability?  
No.

### 🛟 Safety?  
Absolutely not.

### 🚀 Speed?  
**Terrifyingly yes.**

---

> Benchmarks were done using proper hardware, proper testing, and improper software.  
> No undefined behaviors were harmed during testing. Only encouraged.

> ⚠️ Running UB mode for too long may summon Eldritch compiler errors and/or open TCP connections to Cthulhu.
---

## 🚨 Disclaimer

**This project is a high-speed conceptual art piece.**  
Tachyon is not safe. Tachyon is not portable. Tachyon is not enterprise-ready.  
If you're looking for stability, standards compliance, or long-term support — run. Now.  
This is **undefined behavior as a feature**, not a bug.

> Use in production only if your production is a black hole experiment or a meme farm.

---

## 🤔 What is this?

**Tachyon** is a web server built with one goal:  
**to be so fast it causes UB in space-time itself.**

- Undefined behavior? ✅
- Memory safety? 🛑
- Zero-copy everything? ✅
- Actually zero logic sometimes? ✅
- Benchmarks? Nah, it's faster than the profiler.

It’s held together with `unsafe`, bad ideas, and enough `#[inline(always)]` to make your CPU scream in branch-mispredicted agony.

---

## 🌈 Features

- Blazing fast HTTP parser using dark SIMD magic.
- Custom io_uring engine, hand-fed with raw syscalls.
- Core affinity? Yes. Hyperthreading? No idea.
- No dependencies* (except the dozen that make it work).
- Built-in Easter eggs that may or may not serve Rickrolls.
- "Works on my machine" certification.

---

## 🧪 Why?

Because standards are for people who want their software to work.  
This project exists for one reason: **fun**.  
It's a playground for experiments, dark rituals, and "can I get 8M RPS with this hack?"

---

## ⚠️ Requirements

- Linux 6+ with `io_uring`.
- Brain damage (optional but helps).
- Willingness to debug kernel panics with `perf`.

---

## 🐇 Easter Eggs?

Yes. Find them.  
No hints.  

---

## 📦 Building

```bash
cargo build --release
