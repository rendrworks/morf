# WGSL coverage

What a configuration's Lua shader can express, against what WGSL can, and the
ordered plan for closing the gap.

The compiler itself is built — see `PLAN_LUA_SHADER.md`. This is the tracking
document for the *language surface*: which types, operators, statements and
builtins are reachable from Lua today, which are not, and in what order the
missing ones are worth adding.

---

## 1. What "100% coverage" should mean

Not all of WGSL. WGSL is a language for vertex, fragment and **compute**
shaders, and a compute dialect cannot mean anything here: a per-node fragment
shader has no workgroup to synchronise, no storage buffer to write, and no
dispatch to be part of. Aiming at literal totality would mean building
`workgroupBarrier` for nobody.

The coherent target is **everything a fragment shader can express**, and against
that the gap is real and finite. Section 6 is what is out of scope and why, so
that "not done" and "not applicable" never get confused for one another.

Counts below are measured against `naga 30.0.1`, which is what actually accepts
our output — `MathFunction` has **79** variants, plus derivatives and the four
relational functions.

---

## 2. Where it stands

| area | have | missing | note |
|---|---|---|---|
| types | `f32` `i32` `u32` `vec2` `vec3` `vec4` `bool` `mat2` `mat3` `mat4` `array<T, N>` | `f16`, structs | §4.5 |
| operators | everything Lua has except `..` | — | done |
| statements | `local` assign `if` `while` `for` `repeat` `break` `return` | `continue`, `discard` | §4.3 |
| functions | entry point, helpers (monomorphised) | — | done |
| math builtins | **55 of 79** | 24 | §3 |
| derivatives | `dpdx` `dpdy` `fwidth` | the coarse/fine variants, deliberately | §4.4 |
| relational | `all` `any` `isNan` `isInf` | bool vectors to fold | §4.4 |
| textures | one implicit source via `texture(uv)` | everything else | §5 |

Two of the 55 are `select` and `texture`, which are not `MathFunction`s — the
math count proper is 53 of 79 after W3. Of the 26 still missing, 18 are the
packing family in §3.4, which has nothing here to pack for.

---

## 3. The 43 missing builtins

Exact, from naga's own enum. Grouped by whether they are worth having.

### 3.1 Worth adding — ordinary shader arithmetic (13) — **W1 done**

- [x] `saturate` — `clamp(x, 0, 1)`, written constantly
- [x] `trunc`
- [x] `inverseSqrt` — the fast reciprocal length every normalisation wants.
      Spelled `inversesqrt` and `inverse_sqrt`: WGSL writes it one way and
      GLSL, which is what a shader author has read more of, writes it the other
- [x] `fma`
- [x] `faceForward`
- [x] `refract` — we had `reflect` and not this, which was an odd pair to split
- [x] `asinh` `acosh` `atanh` — completing the set already half-present
- [x] `quantizeToF16`
- [ ] `modf` `frexp` — return a struct in WGSL, so they moved to W5 (§4.5)
- [ ] `ldexp` — takes an `i32` exponent, so it moved to W3 (§4.2)

**What W1 caught.** `refract` is the only one of these with a shape of its own —
two vectors and a scalar ratio — and the emitter widened its scalar to the
call's vector type, producing WGSL naga rejects. That is the worst failure this
compiler can produce, because the author gets a validation error with no line
number; the fix was to list the arguments that are *meant* to be a different
type beside `select`'s condition. The GPU test in §8.3 is what found it, which
is the argument for that test existing at all.

### 3.2 Worth adding — matrix (4) — **W2 done**

- [x] `transpose`
- [x] `determinant`
- [x] `inverse`
- [ ] `outer` — outer product, `vecN * vecM` to a matrix. Left out: it is the
      one matrix operation nobody writes by hand, and it needs a non-square
      result the type set does not have.

### 3.3 Worth adding — integer and bitwise (8) — **W3 done**

- [x] `countTrailingZeros` `countLeadingZeros` `countOneBits` `reverseBits`
- [x] `extractBits` `insertBits` — the latter is the one builtin in this
      language taking four arguments, which the first attempt got wrong
- [x] `firstTrailingBit` `firstLeadingBit`

### 3.4 Low value here (18)

Packing and unpacking exist to get data in and out of buffers in a compact
form. A shader here has no buffer of its own to read: its inputs are a handful
of uniforms the host wrote. They are listed so that "missing" is not mistaken
for "overlooked", and each becomes worth having the day §5.3 lands.

- [ ] `pack4x8snorm` `pack4x8unorm` `pack2x16snorm` `pack2x16unorm`
      `pack2x16float` `pack4xI8` `pack4xU8` `pack4xI8Clamp` `pack4xU8Clamp`
- [ ] `unpack4x8snorm` `unpack4x8unorm` `unpack2x16snorm` `unpack2x16unorm`
      `unpack2x16float` `unpack4xI8` `unpack4xU8`
- [ ] `dot4I8Packed` `dot4U8Packed`

---

## 4. Language features

### 4.1 Matrices — `mat2x2` … `mat4x4` — **W2 done**

**The single highest-value gap.** Rotation is the first thing anyone reaches
for in a shader, and before this it could only be written by expanding the
arithmetic by hand.

- [x] `Type::Mat2`, `Mat3`, `Mat4` (square only; `matCxR` still unbuilt)
- [x] Constructors from columns *and* from every component at once — WGSL
      accepts both, and both get written: columns are how a rotation is
      composed, the flat form is how one gets pasted out of somebody else's
      shader
- [x] `matrix * vector`, `vector * matrix` (the row form, which WGSL defines
      and which is not a mistake), `matrix * matrix`, `matrix * scalar`
- [x] `transpose` `determinant` `inverse`
- [x] Uniform layout: a `mat3x3` is three `vec4`-aligned columns — forty-eight
      bytes, not thirty-six — asserted by a test on both sides
- [ ] Column indexing `m[0]`, which needs §4.6

**Two rules that had to be stated rather than inherited.** A matrix is
deliberately *not* "numeric": `m * v` is a linear map applied to a vector, not a
componentwise multiply, so routing it through the same path would make `m + v`
silently mean something. And a scalar never widens into a matrix — the emitter
widens scalars against vector calls, which is right almost everywhere and would
produce `mat2x2<f32>(0.5)` here, which is not even legal WGSL.

Non-square types can wait; nobody writes `mat2x3` by hand.

### 4.2 Integers and bitwise — **W3 done**

- [x] `Type::U32`, and `i32` promoted from loop-counter-only to a real type
- [x] **Abstract integer literals.** The interesting part, and it went the way
      WGSL itself does it: an integer literal has no type until something asks.
      `1 / 2` is `0.5` because nothing in it asks for a whole number, and
      `1 << 2` is four because a shift does. `Type::defaulted` commits an
      undecided literal to `f32` at the points where a type has to stop being
      undecided — a local's declared type, a loop bound, a constructor argument.
- [x] `& | ~ << >>`
- [x] `bitcast_f32` / `bitcast_i32` / `bitcast_u32` — spelled with the target in
      the name, because Lua has no `bitcast<T>` syntax to put it in
- [x] Conversions: `f32(x)`, `i32(x)`, `u32(x)`
- [x] §3.3's eight builtins

**Why abstract literals were not optional.** A hash multiplier like
`2654435769` needs thirty-two bits and an `f32` has twenty-four of mantissa. Had
literals stayed floats, there would have been no way to write a hash at all —
and therefore no noise except the `sin(dot(p, k)) * 43758.5453` trick, which is
what the port suite used because it had no choice. `integer_hash_noise_ports`
is the port that could not be written before this, and
`integer_hash_noise_paints` is the same thing through a real adapter.

### 4.3 Statements

- [ ] `continue` — Lua has no `continue`; it would have to be spelled, and the
      obvious spelling is `goto continue`, which the language rejects. Worth
      deciding on a keyword rather than inventing syntax silently.
- [ ] `discard` — meaningful in Surface mode, where the shader owns coverage
- [ ] `switch` — Lua has none; an `if` chain covers it and the emitter could
      recognise the shape. Low value.

### 4.4 Derivatives and relational — **W4 done**

- [x] `dpdx` `dpdy` `fwidth`
- [ ] The `Coarse`/`Fine` variants. Left out: they are a precision hint, the
      plain forms are what anybody writes, and three spellings of one idea is
      three things to keep straight for no gain.
- [x] `all` `any` `isNan` `isInf`. `all` and `any` fold a single bool for now —
      WGSL folds a bool *vector*, and this language has no such type until
      comparisons on vectors exist.

Derivatives mattered more than they looked: `field.wgsl` antialiases its own
edges with `fwidth`, and before this a configuration's shader could not — so a
shader that drew its own shape had no way to soften it at the resolution it was
actually being drawn at. That asymmetry is closed.

**Uniformity — one check, not two.** WGSL forbids a derivative under non-uniform
control flow, the same rule `texture` already lived under, and both are refused
by the same walk in `validate.rs` naming whichever call it found. A check per
builtin is how the two would have drifted. The diagnostic says why the rule
exists — the call reads neighbouring pixels, which have to have taken the same
path — and what to do instead, and there is a test proving the suggested fix
actually compiles.

### 4.5 Arrays and structs — **W5 mostly done**

- [x] `array<T, N>` with a constant length, written as a Lua list — which is
      what a palette or a convolution kernel wants to be, and what an author
      will write without being told
- [x] Indexing `a[i]`, on arrays, vectors and matrix columns
- [ ] Structs, and with them `modf` and `frexp`. Left for now: nothing else
      wants one, and the two builtins that do are the least-reached-for in the
      language.

**Uniform stride.** In the uniform address space an array's stride is a
multiple of sixteen whatever the element is, so four `f32` occupy sixty-four
bytes rather than sixteen. That is the same rule that rejected the first attempt
at padding the parameter block back in M4, and it is asserted on both sides.

**A gap this step exposed.** `clamp(i32(...), 0, 3)` was refused, because the
componentwise shapes excluded whole numbers — WGSL defines `abs`, `clamp`,
`min`, `max` and `sign` for integers and this language did not. Those five have
their own shape now, kept separate rather than flagged, because `sin` of an
integer really is undefined and letting it through would mean a driver refusing
it with no line number.

### 4.6 Indexing — **W5 done**

`v[i]`, `m[i]` and `a[i]` all work. The note that used to point at `v.x` was
right until arrays existed and then became a wrong answer, which is what it was
flagged as here.

A float index is refused by name rather than rounded: reading the wrong element
silently is how a shader goes subtly wrong and nobody finds out.

---

## 5. Host-side surface

These are not compiler work. Each needs a decision about what the *renderer*
offers before the language can name it.

### 5.1 More than one texture

`texture(uv)` samples one implicit source — the layer underneath, in Effect
mode. A shader that wants a mask, a lookup table or a second layer has no way to
ask for one.

- [ ] Named samplers a configuration declares, bound by the host
- [ ] `textureDimensions`, `textureLoad`, explicit-LOD sampling

### 5.2 Vertex shaders

Nothing in the design assumes fragment. A configuration that could displace
geometry would be a different feature, and a larger one.

### 5.3 Storage buffers

The thing that would make §3.4 worth having. Also the thing that makes a shader
able to keep state between frames, which is a much bigger design question than
it looks — and the reason it is not simply "next".

---

## 6. Out of scope, and why

Not missing. Meaningless in a per-node fragment shader, and building them would
be building for nobody:

- **Compute shaders, workgroups, `workgroupBarrier`, `storageBarrier`.** There
  is no dispatch here to belong to.
- **Atomics.** Nothing to contend over.
- **Pointers and `ptr<>`.** A shader here has no aliasing to express.
- **`f16`.** Wants a device feature and buys nothing at this size.
- **Override declarations / pipeline-overridable constants.** Our parameters are
  uniforms, which is the same capability by a different route, and already
  animatable through the scene.

If any of these ever becomes relevant it will be because §5.2 or §5.3 landed
first, and it should be reconsidered then rather than pre-emptively.

---

## 7. Order

Each step is self-contained: a type, its operators, its builtins, its
diagnostics, its tests. Each ends green on `oslo make verify` and is one commit.

**W1 — the thirteen easy builtins** (§3.1, minus `modf`/`frexp`). No new types,
no new syntax; a table entry and a test each. Gets the arithmetic surface from
34/79 to 45/79 in an afternoon and makes the next steps' tests easier to write.

**W2 — matrices** (§4.1, §3.2). The highest-value gap. Uniform layout is the
part that will bite: a `mat3x3` is not nine floats, and the packer and a test
have to say so together.

**W3 — integers and bitwise** (§4.2, §3.3). The literal-typing rules are the
delicate part — `1 / 2` must stay `0.5` in a float context while `1 << 2` is an
integer — and the diagnostics matter more than the feature.

**W4 — derivatives and relational** (§4.4). Small, and it closes a real
asymmetry: the engine's own shader antialiases with `fwidth` and a user's
cannot. Generalise the uniformity check while here.

**W5 — arrays, indexing, structs** (§4.5, §4.6). Unlocks `modf` and `frexp`,
and lets a configuration write a palette as a Lua table, which is what it will
try to do anyway.

**W6 — `continue` and `discard`** (§4.3). Needs a syntax decision for
`continue`, which is why it is not first despite being small.

**Then** §5, which is host design rather than compiler work and should be
planned separately once there is a use asking for it.

W1 and W2 are most of the value. If this stops after W2, the language can
express rotation, and that is the thing people notice missing.

---

## 8. How each step is checked

The same three, per step, because the ports are the only measure that has
actually caught anything:

1. **Golden tests** — the emitted WGSL for each new form.
2. **Diagnostic tests** — what a wrong use says, with its line number. A feature
   whose error message is bad is a feature people cannot use.
3. **A port that needed it.** W2 is done when a shader that rotates something
   compiles and paints; W3 when integer-hash noise does. A builtin added without
   a shader asking for it is how §3.5 of the shader plan got written short in
   the first place.

---

## 9. Status

- [x] **W1 — the ordinary builtins.** 42 of 79 math functions, from 34.
- [x] **W2 — matrices.** 45 of 79, and rotation is writable.
- [x] **W3 — integers and bitwise.** 53 of 79, and a real hash is writable.
- [x] **W4 — derivatives and relational.** A shader can antialias its own edge.
- [x] **W5 — arrays and indexing.** Structs deferred; nothing wants one yet.
- [ ] W6 — `continue` and `discard`
