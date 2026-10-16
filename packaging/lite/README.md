# BlockForge Lite

A Java agent (`-javaagent:blockforge-lite.jar`) that makes Minecraft run smoother on slow computers. It is not a mod:
it works the same for vanilla, Fabric, Quilt, Forge and NeoForge, needs no mod file and no mod loader, and changes
nothing the server can see. It is separate from the fast-start agent in `../boost`; the two can be used together.

Build: `./build.sh` (needs a JDK 11 - 25 that can still compile for Java 8; it writes `blockforge-lite.jar` and runs
the self-test). The jar is checked in, like the boost jar, so building the launcher needs no JDK. After changing
anything under `src/`, run `build.sh` and commit the new jar.

Self-test: `java -cp blockforge-lite.jar com.blockforge.lite.SelfTest` (exit code 0 = passed). It needs no game and no
display. It builds small stand-ins of every patch target, runs them through the agent, loads them (so the JVM verifies
the changed code) and runs them, and it checks that every kind of trouble leaves a class untouched. CI should run it on
Linux, Windows and macOS with Java 8, 17, 21 and 25 (it was run here on Linux with Java 8, 17, 21 and 25, also with
`-Xverify:all`).

Matcher: `java -cp blockforge-lite.jar com.blockforge.lite.Check [--options OPTS] client.jar [more.jar ...]` runs every
patch over a game jar without starting the game (the same code path as at a game start, including the check of the
changed code) and prints the report lines: which patches would apply, which would skip and why. Exit code 1 when a
patch failed. For CI: run it on every new release and snapshot jar (from the version manifest) and compare with the
previous release; it takes about 1.5 s per jar.

For people writing patches: `-Dblockforge.lite.dump=DIR` writes every class a patch looked at to DIR, as the agent got
it (`NAME.before.class`, i.e. after the mod loader's own changes) and as changed (`NAME.after.class`), for `javap`.

## Options

`-javaagent:/path/blockforge-lite.jar=OPTIONS`, options separated by commas (so a path can't contain a comma):

| Option | Meaning |
|---|---|
| (none) | the conservative set (level 1, today all patches) |
| `quiet` | no summary line; problems are still printed |
| `off=ID;ID` | these patches stay off (one per launcher setting) |
| `only=ID;ID` | only these patches (of any level); `off=` still wins |
| `level=1` / `level=2` | conservative (default) / also the balanced patches |
| `report=FILE` | one line per patch, tab separated: id, `applied`/`skipped`/`failed`, class, reason. Lines starting with `#` are comments; one says how much time the agent took |
| `mods=FILE` | the game's mods, one file name or mod id per line (the launcher knows them best). An empty list (or an empty `modids=`) tells the agent there are no mods; patches that are only safe without mods (`slim-sections` on level 1) need that or Minecraft's own main class |
| `modids=ID;ID` | the same, inline |
| `targets=FILE` | optional: lines `patch-id runtime/class/Name` (several per patch allowed); for those patches only these classes are looked at. The launcher can work them out from the Mojang/intermediary/SRG mappings it has; it saves looking at every class of an obfuscated vanilla game |
| `strict` | match only classes whose runtime name is Mojang's (NeoForge, Forge 1.17+, 26.x) or named by `targets=`, and only Minecraft 1.17+ class files. Without it, obfuscated (vanilla <= 1.21.11) and intermediary (Fabric/Quilt) classes are matched by the shape of their code too |
| `verbose` | prints every decision as it happens |

Unknown options are ignored. Off switches for the player, without changing the launcher: `-Dblockforge.lite=off`, or
`-Dblockforge.lite.off=FILE` (off while that file exists).

What it prints (every line starts with `[BlockForge Lite]`):
- `[BlockForge Lite] on: 3 patches applied (idle-fps slim-sections short-fade), 2 skipped` once, when every patch has
  decided or a minute after the start (later decisions go to the report only);
- `[BlockForge Lite] problem: ...` when something went wrong (the game carries on without that patch);
- `[BlockForge Lite] off` when an off switch is set.

## Patches

Level 1 patches are on by default and keep the game's behaviour exactly; level 2 ("balanced", only with `level=2`)
ones change something small that a player could notice, said in the table.

| Id | What it does | Where it applies | Stays off when |
|---|---|---|---|
| `idle-fps` | While the window is minimised (or is a full screen the player switched away from), draws 10 frames a second, as Minecraft 1.21.2 and newer do by themselves. | 1.8 - 1.21.1 (LWJGL 2 and 3) | Dynamic FPS, FPS Reducer, VulkanMod |
| `no-world-gc` | Removes the two `System.gc()` calls that pause the whole game for a full garbage collection each time a world is opened or left and at every portal. The calls for "out of memory" stay, and so does Java's own use for native buffers (unlike `-XX:+DisableExplicitGC`). | 1.7 - 1.12 | - |
| `slim-sections` | Drops the debug thread guard (`ThreadingDetector`, 5 objects, 128 bytes) from every chunk section container; acquire/release stop using it, as Lithium's `chunk.no_locking` does. Vanilla never trips the guard; a buggy mod that changes a chunk section from two threads at once would get a silently broken chunk instead of the guard's crash report, so on level 1 it is only applied when no mod can be there: Minecraft's own main class started the game, or the launcher's list (`mods=` / `modids=`) is given and empty (any entry counts as a mod). With mods it needs `level=2` (report: "mods installed: kept the guard"). | 1.18 - 26.x | FerriteCore (does it already) |
| `short-fade` | The Mojang loading screen fades out in 0.5 s instead of 2 s after loading (and fades in for 0.25 s instead of 1 s after an in-game reload). Loading itself is unchanged. | 1.20 - 26.x | a mod changed the loading screen |
| `outline-skip` | Skips the full-screen pass that draws the glowing-entity outline layer when nothing glows (the layer is empty then, so the picture is identical), as Minecraft 26.3 does. | 1.20 - 1.21.1, without a mod loader hooking the renderer (vanilla, Fabric/Quilt without their APIs) | OptiFine, VulkanMod, a mod or loader hook in that code |
| `worker-priority` | Not a class patch (a "tweak"): on 4 or fewer logical processors, the game's `Worker-Main-N` threads (world generation, light, chunk meshing, loading) run one step below normal priority, so the render and server threads get the processor first. Takes effect on Windows (BELOW_NORMAL); HotSpot ignores Java priorities on Linux and macOS. | every version with `Worker-Main` threads (1.14+) | - |
| `lazy-collision-shape` | Every collision check (entities moving, "is there room here", items, on the server and the client) turned the entity's box into a voxel shape up front (about 200 bytes); it is only needed when the check meets a partial block (slab, stairs, fence...). The shape is now made the first time it is needed, from the same box: the same value, made later or not at all. | 1.20.1 - 26.x (`BlockCollisions`; older versions not checked) | a mod changed or reads that code |
| `empty-effect-lists` | Moving entities collect the effects of the blocks they touch per movement step; copying the ten (almost always empty) per-type lists into the final list made a new empty array each time. Empty lists are now skipped; the effects and their order are the same. | 1.21.2 - 26.x (`InsideBlockEffectApplier$StepBasedCollector`) | Lithium, Canary, Radium (do it already) |
| `shared-state-tables` | Each block state's neighbour table (property, value -> state) had its own row and column index maps; tables of states with the same properties now share them (Guava's ArrayTable copy). Every lookup gives the same state as before; applied only when the game reads the table through `get` alone. | 1.20 - 1.21.1 (`StateHolder`; newer versions use arrays) | FerriteCore (replaces these tables) |
| `short-spin` (level 2) | The frame limiter of 26.x sleeps until shortly before the frame is due and busy-waits the rest (0.5 ms plus the average oversleep, up to 2.5 ms per frame, a core spinning). Now 0.1 ms plus at most 1 ms. A frame can come up to about 0.5 ms later when the system oversleeps more than usual. Only matters when the frame rate is limited (menus, or a limit below "unlimited"). | 26.1 - 26.x (`FramerateLimiter`) | a mod changed the limiter |
| `sound-no-wait` (level 2) | Starting a sound no longer makes the game wait for the sound thread (which may be decoding music): the channel handle is handed out at once and gets its channel on the sound thread before any of its other tasks run. When all sound channels are busy, a sound that can't play counts as playing for a second (it stays silent, as before) instead of "not started"; for music that can mean a "now playing" notice for a song that is not heard. | 1.20.1 - 26.x (`ChannelAccess`; needs Java 11+ class files; older versions not checked) | Sound Physics Remastered, AmbientSounds, Presence Footsteps |
| `quiet-music` (level 2) | Since 1.21.2 music starts "silently" when its volume is zero, and the sound thread keeps decoding it; now muted music is skipped like every other muted sound. Turned up again, music starts with the next song instead of in the middle of the silent one. | 1.21.2 - 26.x (`SoundEngine.play`) | Sound Physics Remastered, AmbientSounds |

Where a target looks different (another version, a mod changed it), the patch does nothing and the report says why.

Where they apply (lab instances, real starts, with full bytecode verification): vanilla 1.21.1, Fabric 1.21.1 and
Quilt 1.21.1 get idle-fps, slim-sections, short-fade and outline-skip; NeoForge 1.21.1 gets idle-fps, slim-sections and
short-fade (its LevelRenderer has the loader's hooks); Forge 1.20.1 idle-fps and short-fade (slim-sections when a world
loads); 26.3 vanilla and Fabric slim-sections and short-fade; Forge 1.12.2 and vanilla 1.8.9 idle-fps and no-world-gc.
`Check` on the jars: 1.20.1 and 1.21.1 as 1.21.1; 1.21.11, 26.3, 26.4-snapshot-4: slim-sections and short-fade.
Since slim-sections needs "no mods" on level 1, under a mod loader it is applied only with `level=2` or an empty
`mods=` / `modids=` list; run `Check` with `--options modids=` to see the vanilla case.

The engine patches (lab, 4 cores, all with `level=2` and full bytecode verification, world `labworld` with walking,
swimming and flying where the instance has it): vanilla 1.21.1, Fabric 1.21.1, Quilt 1.21.1 and NeoForge 1.21.1 get
lazy-collision-shape, shared-state-tables and sound-no-wait (on intermediary names under Fabric/Quilt); 26.3 vanilla and
Fabric get lazy-collision-shape, empty-effect-lists, short-spin, sound-no-wait and quiet-music; Forge 1.20.1 (title
screen only, the instance has no world) shared-state-tables and sound-no-wait. No errors in any game log. `Check` on
the jars: lazy-collision-shape and sound-no-wait 1.20.1 - 26.4-snapshot-4, shared-state-tables 1.20.1 and 1.21.1,
empty-effect-lists and quiet-music 1.21.11 - 26.4-snapshot-4, short-spin 26.1 - 26.4-snapshot-4. Fabric 1.21.11 has no
intermediary name for empty-effect-lists' class in the agent (the launcher can pass it with `targets=`).

## Measured

Lab: Linux, Xvfb with Mesa llvmpipe (software OpenGL, so the GPU work is CPU work here), 4 cores (taskset), nice 10,
interleaved runs with and without the patch (`off=ID`), same world. Numbers are per run; no real GPU, no Windows or
macOS.

| Patch | What was measured | With | Without |
|---|---|---|---|
| idle-fps | 1.21.1, title screen, window marked minimised, 3 + 3 runs: frame rate / game CPU | 10 fps / 0.34 - 0.41 cores | 50 - 57 fps / 1.65 - 1.72 cores |
| no-world-gc | Forge 1.12.2 (Java 8), open a world, save and quit, 3 + 3 runs: full GCs from `System.gc()` and their pauses | 0 | 4, 1.87 - 2.01 s in total (heap 130 - 350 MB; bigger with mods) |
| slim-sections | live heap 40 s after joining (`jcmd GC.class_histogram`), 3 + 3 runs | 26.3: 0 guard objects; 1.21.1: 313 - 316 MB | 26.3: 197 k - 292 k guards (5 objects, 128 B each: 25 - 37 MB); 1.21.1: 65,856 guards, 322 - 323 MB (8 MB) |
| short-fade | 1.21.1 start, time the red Mojang screen is visible (one pixel sampled every 25 ms), 3 + 3 runs | 4.35 - 4.89 s | 6.82 - 7.10 s (the constants account for 1.5 s; the rest of the difference is not explained) |
| outline-skip | 1.21.1 in a world, median frame rate over 20 s (Mesa's HUD), 3 + 3 runs | 10.0, 9.8, 8.6 fps | 6.2, 7.3, 4.3 fps (software rendering makes the full-screen pass expensive; on a real GPU it is one blended full-screen pass per frame, not measured) |
| worker-priority | 1.21.1 in a world on 4 cores (`jcmd Thread.print`) | Worker-Main threads priority 4 | 5. The effect on frame and tick times needs Windows and was not measured here |
| shared-state-tables | 1.21.1 vanilla, live heap 20 s after the title screen (`jcmd GC.class_histogram`), 3 + 3 runs | 193.4 - 194.5 MB (Guava collections 2.8 MB) | 210.6 - 211.7 MB (Guava collections 13.9 MB): 17 - 18 MB more, mostly 280 k map entries, 51 k index maps and their arrays |
| lazy-collision-shape | sampled allocation (JFR) over 56 s in a world, walking/swimming/flying, 3 + 3 runs | 1.21.1: no shapes from collision checks, server thread 47.1 - 47.6 MB/s; 26.3: none | 1.21.1: 0.8 - 1.6 MB/s of shapes, server thread 49.2 - 50.5 MB/s; 26.3: 0.24 - 0.39 MB/s (A2's earlier 26.3 walk on land: 1.8 MB/s). Tick time and GC count: no difference beyond noise |
| empty-effect-lists | 26.3, same runs as above | no `Object[]` from flushStep | 0.43 - 0.97 MB/s of empty arrays. flushStep's share of the server thread's samples (0.7 - 1.0 %) did not change measurably |
| short-spin | 26.3 title screen (menu limit 60 fps, 427x240 window), Render thread CPU over 70 s, 3 + 3 runs | 0.124 - 0.134 cores | 0.142 - 0.150 cores (on Windows the oversleep, and so the spin, is usually longer; not measured) |
| sound-no-wait | 26.3 in a world, same runs as lazy-collision-shape: Render thread waits of 1 ms or more inside `SoundEngine.play` (JFR) | none | 7 - 9 waits per 56 s, 20 - 34 ms in total, longest 3 - 14 ms (the analysis saw up to 0.5 s on busy cores). Real audio output was not tested (the lab has no sound device; OpenAL ran on its null device) |
| quiet-music | 26.3 title screen with music volume 0, sound thread CPU over 70 s, same runs as short-spin | 0.000 cores | 0.005 - 0.006 cores (decoding music nobody hears; more on a slow processor) |

The agent's own cost, from the report's time line (real starts without bytecode verification, one run each): 0.10 -
0.33 s in total over a whole start (1.21.1 vanilla 0.33 s, Fabric 0.15 s, NeoForge 0.10 s, Forge 1.20.1 0.19 s, 26.3
0.16 - 0.24 s, Forge 1.12.2 0.22 s), against starts of 20 - 50 s. Most of it is the first use of ASM on the 3 - 4 big
target classes (Minecraft, LevelRenderer) while the game's code is still cold, plus looking at every class in no package
on an obfuscated vanilla game; `targets=` or `strict` avoid the latter.

## How it stays safe

- **Isolated.** Only small classes are loaded from the agent jar on the game's class path. The patch code and its ASM
  9.10.1 (`lib/`, BSD licence in `lib/LICENSE-ASM.txt`) are kept in a zip inside the jar and loaded by a private class
  loader whose parent is the JDK's loader, so mod loaders and mods (which bring their own ASM) never see them, and the
  patch code never loads a game class.
- **Loader-neutral changes.** Changed game code calls only the JDK, LWJGL and the game, plus private static methods and
  fields the patch adds to the same class (named `blockforge$lite$...`, easy to spot in a crash report). Nothing is
  shared through the system class loader, so it works the same under Fabric's Knot, Forge/NeoForge's module layers and
  LaunchWrapper.
- **Found by what the code does, not by its name.** Names differ between vanilla (obfuscated before 26.1),
  Fabric/Quilt (intermediary), Forge (SRG methods) and NeoForge (Mojang names). Each patch knows the stable class names
  (Mojang and intermediary) and otherwise looks at classes in no package (obfuscated vanilla); it first checks the
  constant pool for a few texts (cheap, no parsing), finds the methods it needs by scanning the raw bytes for the
  instructions it expects, and then checks the shape of the code around its target exactly (instructions,
  descriptors, constants, how often they occur). Anything unexpected: the class is left alone, and the report says
  `skipped` and why.
- **Mods first.** A patch whose job a performance mod already does stays off when that mod is installed (found in the
  `mods=`/`modids=` list, in the game's `mods` folder, or by one of its classes). A target method that Mixin changed
  (merged handlers, callback calls, MixinExtras) is not patched. The agent sees classes after the mod loader's own
  changes on every loader, because those loaders define the classes they changed.
- **Checked before use.** Only the methods a patch changes are rewritten; every other method is copied byte for byte.
  Frames are never recomputed (that would need to load game classes): patches only insert instructions with no net
  stack effect, swap constants, or add small methods with hand-written frames. The result is read again and the
  changed methods are run through ASM's verifier. A patch that throws or fails the check is dropped and the class
  loads as it was (or with the other patches only). Errors never reach the game.
- **Cheap.** Classes outside `net/minecraft/`, `com/mojang/blaze3d/` and the default package are passed by on their
  name; named Minecraft classes other than the known targets are passed by too; the rest are filtered on their
  constant pool. Measured on real starts: 0.10 - 0.33 s in total over a whole start (most of it the first use of
  ASM), see "Measured". Patches that found their place stop looking. With a mod loader, copies of game classes in the
  application class loader (Fast start can preload some there; they are never used) are ignored.
- **Newer Java.** Class files newer than ASM knows are left alone (reported as skipped).

## Adding a patch

Everything lives in `src/core/com/blockforge/lite/core/` (package-private, Java 8 source level). A new patch is two new
files and two one-line additions; nothing else needs to change.

1. **Read the target** in the decompiled sources (`~/bf-lab/decomp`) and `javap -c -p` of every version you want to
   support (obfuscated jars too), and write down the exact instructions you change plus what makes the class unique (a
   string constant, a JDK/LWJGL call). Prefer changes that need no new stack map frames: insert a call to a static
   method with the same stack effect (`x -> x`), swap a constant, remove a call with no stack effect, or replace an
   instruction window with one of the same stack effect. A new method with branches needs hand-written frames (see
   `IdleFps.glfwHelper`, `OutlineSkip.helper`); never ask ASM to compute frames.
2. **Write `MyPatch extends Patch`**:
   ```java
   MyPatch() {
       super("my-patch", 1, "what it does, in plain words",
             new String[][] {{"net/minecraft/SomeOwner", "someName"}},   // constant-pool texts; any one set must be all present
             new String[] {"sodium", "embeddium"},                       // Mods.KNOWN keys: stay off when installed
             "net/minecraft/client/Target", "net/minecraft/class_1234"); // Mojang + intermediary class names
   }
   void apply(Target t, Ctx ctx) throws Skip { ... }
   ```
   In `apply`:
   - find code with `Target`: `t.calls(opcode, owner, name|null, desc)` (`[MethodNode, call]` pairs),
     `t.fieldUsers(owner, name, desc)`, `t.stringUsers(text)`, `t.method(name, desc)`, `t.methodsNamed(name)`,
     `t.allMethods()` (small classes only), `t.fieldAccess(name, desc)`, `t.name`, `t.version`, `t.isInterface()`.
     Only these methods are parsed with ASM; the class's other methods are copied byte for byte.
   - check the exact shape with `Code`: `prev/next/first` (real instructions), `before(n, opcodes...)`,
     `after(n, opcodes...)`, `isCall`, `isVar`, `isInt`, `calls(m, ...)`, `ldcs(m, value)`, and refuse methods that
     `Code.changedByMixin(m)` (mixin handlers, callback infos, `@MixinMerged`) or, where it matters,
     `Code.callsModLoader(m)` (Forge/NeoForge/Fabric/Quilt hooks).
   - not the target: `throw Skip.notTarget("...")` (silent, keeps looking). The target but different:
     `throw Skip.because("...")` (reported as skipped, the patch stops looking).
   - change `MethodNode`s and call `t.changed(m)`; add helpers with `t.add(methodNode)` and `t.addField(fieldNode)`
     (private static synthetic, names starting with `Code.PREFIX`, `blockforge$lite$`). Helpers may call only the JDK,
     LWJGL and the game (never a class of the agent jar). If they branch and `t.needsFrames()`, write F_SAME/F_FULL
     frames.
   - other exceptions are failures: the class loads without the patch and a problem line is printed.
   - override `once()` to return false for a patch with several target classes; `ctx.level` tells level 1 or 2.
   - something that is not a class change (threads, timers) is a `Tweak` instead: `super(id, level, what,
     conflictsWith)` and `String start()` (start a daemon thread if needed; return null when on, or why not). See
     `WorkerPriority`.
3. **Register it** in `Engine.all()` (one line). New mods to detect go into `Mods.KNOWN`
   (`{key, display name, file-name word, class file paths...}`).
4. **Write `MyPatchTest extends PatchTest`** with one method per test (no arguments, not private) and add its class to
   `Tests.PATCH_TESTS` (one line). Build stand-ins with `writer()` (ASM, frames computed for the stand-in only) and
   `constructor(w)`, load them through the agent with `load(factory.apply("quiet,report=" + reportPath("x")), classes)`,
   run them with reflection, check with `expect(ok, what)` and `line(report, "my-patch")`. Test the working case,
   old/new class versions where relevant (`runs(V21)` tells whether this JVM can load them), and at least one shape it
   must leave alone (mixin-changed, a different version).
5. `./build.sh` (builds and runs the self-test); `java -cp blockforge-lite.jar com.blockforge.lite.Check` on the client
   jars of every version (`~/bf-lab/research-r2/jars`) shows where it applies.
6. **Run the lab instances** with `report=FILE` (and `-Dblockforge.lite.dump=DIR` to see the bytes): applied where
   expected, skipped elsewhere, no errors in the game log, also with full bytecode verification
   (`-XX:+BytecodeVerificationRemote`, `-Xverify:all` on Java 8).
7. **Measure**: interleaved runs with and without it (`off=my-patch`). Keep it only if it helps measurably and is safe.
