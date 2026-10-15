/*
 * BlockForge - a fork of ATLauncher (https://github.com/ATLauncher/ATLauncher)
 *
 * This program is free software: you can redistribute it and/or modify
 * it under the terms of the GNU General Public License as published by
 * the Free Software Foundation, either version 3 of the License, or
 * (at your option) any later version.
 *
 * This program is distributed in the hope that it will be useful,
 * but WITHOUT ANY WARRANTY; without even the implied warranty of
 * MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE. See the
 * GNU General Public License for more details.
 *
 * You should have received a copy of the GNU General Public License
 * along with this program. If not, see <http://www.gnu.org/licenses/>.
 */
package com.blockforge.launcher.mclauncher;

import java.io.IOException;
import java.nio.charset.StandardCharsets;
import java.nio.file.DirectoryStream;
import java.nio.file.Files;
import java.nio.file.Path;
import java.nio.file.Paths;
import java.util.ArrayList;
import java.util.List;
import java.util.Locale;
import java.util.Optional;

import com.blockforge.launcher.App;
import com.blockforge.launcher.data.Instance;
import com.blockforge.launcher.utils.Java;
import com.blockforge.launcher.utils.LightMode;
import com.blockforge.launcher.utils.PcProfile;

/**
 * Gathers what {@link AutoMemory} needs from an instance and this computer, and decides the memory for a launch:
 * automatic, or what the player set.
 */
public final class InstanceMemory {
    private InstanceMemory() {
    }

    /** The memory for one launch. */
    public static final class Decision {
        public final boolean automatic;
        public final int heapMb;
        public final List<String> warnings;
        /** the automatic choice, also when the player's own setting is used (for showing what it would be) */
        public final AutoMemory.Result auto;
        public final AutoMemory.Game game;

        Decision(boolean automatic, int heapMb, List<String> warnings, AutoMemory.Result auto, AutoMemory.Game game) {
            this.automatic = automatic;
            this.heapMb = heapMb;
            this.warnings = warnings;
            this.auto = auto;
            this.game = game;
        }

        /** if the player should be told on the Play screen that the game is short of memory */
        public boolean isShort() {
            return automatic && auto.isShort();
        }
    }

    /** If this instance uses automatic memory. */
    public static boolean isAutomatic(Instance instance) {
        boolean launcherAutomatic = App.settings == null || !Boolean.FALSE.equals(App.settings.automaticMemory);
        return GameMemory.usesAutomatic(instance.launcher.maximumMemory, instance.launcher.automaticMemory,
            launcherAutomatic);
    }

    /** Decides the memory for starting this instance with the Java in this folder. Reads a few folders. */
    public static Decision decide(Instance instance, String javaPath) {
        AutoMemory.Game game = gameOf(instance);
        AutoMemory.Pc pc = pcNow(javaPath);
        AutoMemory.Result auto = AutoMemory.choose(pc, game);

        if (isAutomatic(instance)) {
            return new Decision(true, auto.heapMb, new ArrayList<>(auto.warnings), auto, game);
        }

        int configured = Optional.ofNullable(instance.launcher.maximumMemory).orElse(App.settings.maximumMemory);
        GameMemory.Plan plan = GameMemory.plan(configured, game.packMinimumMb, (int) Math.max(0, pc.totalMb),
            !pc.java32Bit);
        return new Decision(false, plan.heapMb, new ArrayList<>(plan.warnings), auto, game);
    }

    /**
     * The automatic memory for showing in the settings, such as "about 3 GB for a modern Fabric game with 6 mods":
     * for the given instance, or a plain modern game when there's none. Free memory isn't looked at, as it changes
     * all the time. Reads a few folders, so not on the Swing thread.
     */
    public static String describeAutomatic(Instance instance) {
        AutoMemory.Game game = instance == null ? AutoMemory.Game.vanilla(null) : gameOf(instance);
        boolean java32 = instance == null ? !Java.is64Bit() : is32BitJava(instance.getJavaPathQuietly());
        AutoMemory.Result result = AutoMemory.choose(
            new AutoMemory.Pc(LightMode.totalRamMb(), -1, Runtime.getRuntime().availableProcessors(), java32), game);
        return "about " + AutoMemory.describeMb(result.heapMb) + " for " + AutoMemory.describeGame(game);
    }

    /** What an instance is, as far as memory goes. */
    public static AutoMemory.Game gameOf(Instance instance) {
        Path root = instance.getRoot();
        AutoMemory.Loader loader = AutoMemory.Loader
            .of(instance.launcher.loaderVersion == null ? null : instance.launcher.loaderVersion.type);

        int recommended = 0;
        if (instance.launcher.curseForgeManifest != null && instance.launcher.curseForgeManifest.minecraft != null
            && instance.launcher.curseForgeManifest.minecraft.recommendedRam != null) {
            recommended = instance.launcher.curseForgeManifest.minecraft.recommendedRam;
        }
        int minimum = Optional.ofNullable(instance.launcher.requiredMemory).orElse(0);

        return new AutoMemory.Game(instance.id, loader, countMods(root.resolve("mods")),
            hasAny(root.resolve("shaderpacks")), sizeMb(root.resolve("resourcepacks")), recommended, minimum);
    }

    /** This computer right now. */
    public static AutoMemory.Pc pcNow(String javaPath) {
        return new AutoMemory.Pc(LightMode.totalRamMb(), PcProfile.availableMb(),
            Runtime.getRuntime().availableProcessors(), is32BitJava(javaPath));
    }

    /** Mod jars in the mods folder (disabled ones don't count). */
    static int countMods(Path mods) {
        int count = 0;
        if (!Files.isDirectory(mods)) {
            return 0;
        }
        try (DirectoryStream<Path> files = Files.newDirectoryStream(mods)) {
            for (Path file : files) {
                String name = file.getFileName().toString().toLowerCase(Locale.ROOT);
                if ((name.endsWith(".jar") || name.endsWith(".zip")) && Files.isRegularFile(file)) {
                    count++;
                }
            }
        } catch (IOException e) {
            // count what was seen
        }
        return count;
    }

    /** if the folder has any shader pack (a zip or a folder) */
    static boolean hasAny(Path folder) {
        if (!Files.isDirectory(folder)) {
            return false;
        }
        try (DirectoryStream<Path> files = Files.newDirectoryStream(folder)) {
            for (Path file : files) {
                String name = file.getFileName().toString().toLowerCase(Locale.ROOT);
                if (name.endsWith(".zip") || Files.isDirectory(file)) {
                    return true;
                }
            }
        } catch (IOException e) {
            // none
        }
        return false;
    }

    /** total size of the files directly in a folder, in MB */
    static long sizeMb(Path folder) {
        long bytes = 0;
        if (!Files.isDirectory(folder)) {
            return 0;
        }
        try (DirectoryStream<Path> files = Files.newDirectoryStream(folder)) {
            for (Path file : files) {
                if (Files.isRegularFile(file)) {
                    bytes += Files.size(file);
                }
            }
        } catch (IOException e) {
            // what was counted
        }
        return bytes / (1024 * 1024);
    }

    /**
     * If the Java in this folder is a 32-bit one, from its "release" file. When it can't be told, the launcher's own
     * Java is the best guess.
     */
    public static boolean is32BitJava(String javaPath) {
        if (javaPath != null) {
            Path release = Paths.get(javaPath).resolve("release");
            try {
                if (Files.isRegularFile(release)) {
                    for (String line : Files.readAllLines(release, StandardCharsets.UTF_8)) {
                        if (line.startsWith("OS_ARCH=")) {
                            return is32BitArch(line.substring("OS_ARCH=".length()).replace("\"", "").trim());
                        }
                    }
                }
            } catch (IOException | RuntimeException e) {
                // fall through
            }
        }
        return !Java.is64Bit();
    }

    static boolean is32BitArch(String arch) {
        String a = arch.toLowerCase(Locale.ROOT);
        if (a.contains("64")) {
            return false;
        }
        return a.equals("x86") || a.equals("i386") || a.equals("i586") || a.equals("i686") || a.startsWith("arm");
    }
}
