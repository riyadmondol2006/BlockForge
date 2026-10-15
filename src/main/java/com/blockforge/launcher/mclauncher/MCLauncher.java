/*
 * ATLauncher - https://github.com/ATLauncher/ATLauncher
 * Copyright (C) 2013-2022 ATLauncher
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

import java.io.File;
import java.io.IOException;
import java.nio.charset.StandardCharsets;
import java.nio.file.Files;
import java.nio.file.Path;
import java.nio.file.Paths;
import java.util.ArrayList;
import java.util.Arrays;
import java.util.LinkedHashMap;
import java.util.List;
import java.util.Locale;
import java.util.Map;
import java.util.Optional;
import java.util.stream.Stream;

import com.blockforge.launcher.App;
import com.blockforge.launcher.FileSystem;
import com.blockforge.launcher.constants.Constants;
import com.blockforge.launcher.data.DisableableMod;
import com.blockforge.launcher.data.Instance;
import com.blockforge.launcher.data.MicrosoftAccount;
import com.blockforge.launcher.data.QuickPlayOption;
import com.blockforge.launcher.data.json.QuickPlay;
import com.blockforge.launcher.data.minecraft.Library;
import com.blockforge.launcher.data.minecraft.LoggingClient;
import com.blockforge.launcher.managers.LWJGLManager;
import com.blockforge.launcher.managers.LogManager;
import com.blockforge.launcher.network.ErrorReporting;
import com.blockforge.launcher.utils.Java;
import com.blockforge.launcher.utils.OS;
import com.blockforge.launcher.utils.Pair;
import com.blockforge.launcher.utils.Utils;

public class MCLauncher {
    public static final List<String> IGNORED_ARGUMENTS = Arrays.asList("--clientId", "${clientid}", "--xuid",
        "${auth_xuid}");

    public static Process launch(MicrosoftAccount account, Instance instance, Path nativesTempDir,
        Path lwjglNativesTempDir,
        String wrapperCommand, String username) throws Exception {
        return launch(account, instance, null, nativesTempDir.toFile(), lwjglNativesTempDir, wrapperCommand, username);
    }

    private static Process launch(MicrosoftAccount account, Instance instance, String props, File nativesDir,
        Path lwjglNativesTempDir, String wrapperCommand, String username) throws Exception {
        List<String> arguments = getArguments(account, instance, props, nativesDir.getAbsolutePath(),
            lwjglNativesTempDir, username);
        if (wrapperCommand != null && !wrapperCommand.isEmpty()) {
            arguments = wrapArguments(wrapperCommand, arguments);
        }

        // this only writes to the log, so it shouldn't hold up starting the game
        App.TASKPOOL.execute(() -> logInstanceInformation(instance));

        LogManager.info("Launching Minecraft with the following arguments (user related stuff has been removed): "
            + censorArguments(arguments, account, props, username));
        ProcessBuilder processBuilder = new ProcessBuilder(arguments);
        processBuilder.directory(instance.getRootDirectory());
        processBuilder.redirectErrorStream(true);
        processBuilder.environment().remove("_JAVA_OPTIONS"); // Remove any _JAVA_OPTIONS, they are a PAIN

        // Inject user-defined environment variables mixed with any other environment variables
        Map<String, String> env = App.settings.environmentVariables;
        if (!env.isEmpty()) {
            LogManager.info("Injecting environment variables: " + env);
            processBuilder.environment().putAll(env);
        }

        return processBuilder.start();
    }

    private static void logInstanceInformation(Instance instance) {
        try {
            if (instance.launcher.loaderVersion != null) {
                LogManager.info(String.format("Loader: %s %s", instance.launcher.loaderVersion.type,
                    instance.launcher.loaderVersion.version));
            }

            // LWJGL 2 (Minecraft 1.12 and older) asks the xrandr program about the screens on Linux and crashes with
            // an unhelpful error when it isn't installed, which many minimal and Wayland setups don't have
            if (OS.isLinux() && usesLwjgl2(instance) && !Utils.executableInPath("xrandr")) {
                LogManager.warn("This Minecraft version needs the \"xrandr\" program on Linux, but it wasn't found. "
                    + "If the game crashes on start, install xrandr (x11-xserver-utils on Debian/Ubuntu, "
                    + "xrandr on Fedora/Arch).");
            }

            if (instance.hasDisabledJavaRuntime()) {
                LogManager.warn(
                    "The Use Java Provided By Minecraft option has been disabled. If you're experiencing crashes, please enable this option.");
            }

            Path modsFolder = instance.ROOT.resolve("mods");
            if (Optional.ofNullable(modsFolder.toFile().listFiles()).orElse(new File[] {}).length != 0) {
                LogManager.info("Mods:");

                try (Stream<Path> stream = Files.walk(modsFolder)) {
                    stream
                        .filter(file -> Files.isRegularFile(file)
                            && (file.toString().endsWith(".jar") || file.toString().endsWith(".zip")))
                        .forEach(file -> {
                            String filename = file.toString().replace(modsFolder.toString(), "");
                            DisableableMod mod = instance.launcher.mods.parallelStream()
                                .filter(m -> filename.contains(m.file)).findFirst().orElse(null);

                            boolean isCustomAdded = filename.lastIndexOf(File.separator) == 0
                                && (mod == null || mod.userAdded);

                            LogManager.info(String.format(" - %s%s", filename, isCustomAdded ? " (Added)" : ""));
                        });
                }
            }

            if (instance.launcher.mods.stream().anyMatch(m -> m.skipped)) {
                instance.launcher.mods.stream().filter(m -> m.skipped).forEach(m -> LogManager.warn(String.format(
                    "Mod %s (%s) was skipped from downloading during instance installation", m.name, m.file)));
            }

            if (instance.shouldUseLegacyLaunch() && Optional.ofNullable(instance.launcher.disableLegacyLaunching)
                .orElse(App.settings.disableLegacyLaunching)) {
                LogManager.warn(
                    "Legacy launching disabled. If you have issues with Minecraft, please enable this setting again");
            }
        } catch (IOException ignored) {
            // ignored
        }
    }

    private static void warnIfMemoryIsShort(int heapMb) {
        try {
            long availableMb = OS.getSystemInfo().getHardware().getMemory().getAvailable() / (1024 * 1024);
            String warning = GameMemory.availabilityWarning(heapMb, availableMb);
            if (warning != null) {
                LogManager.warn(warning);
            }
        } catch (Throwable t) {
            // only a hint, never worth failing over
        }
    }

    private static boolean usesLwjgl2(Instance instance) {
        return instance.libraries != null && instance.libraries.stream()
            .anyMatch(l -> l.name != null && l.name.startsWith("org.lwjgl.lwjgl:lwjgl:2"));
    }

    private static List<String> wrapArguments(String wrapperCommand, List<String> args) {
        List<String> wrapArgs = new ArrayList<>(Arrays.asList(wrapperCommand.trim().split("\\s+")));

        // wrapper not set
        if (wrapArgs.isEmpty()) {
            return args;
        }

        String wrapArgsKey = "%command%";
        int commandIndex = wrapArgs.indexOf(wrapArgsKey);
        if (commandIndex >= 0) {
            wrapArgs.remove(commandIndex);
            wrapArgs.addAll(commandIndex, args);
            return wrapArgs;
        }

        // make args as a whole string, useful in the case of ''
        String wrapArgsAsWholeStringKey = "%\"command\"%";
        commandIndex = wrapArgs.indexOf(wrapArgsAsWholeStringKey);
        if (commandIndex >= 0) {
            wrapArgs.set(commandIndex, "'" + String.join("' '", args) + "'");
            return wrapArgs;
        }

        // failback to wrap command with the rest of the arguments added in
        wrapArgs.addAll(args);
        return wrapArgs;
    }

    private static List<String> getArguments(MicrosoftAccount account, Instance instance, String props,
        String nativesDir, Path lwjglNativesTempDir, String username) {
        StringBuilder cpb = new StringBuilder();
        boolean hasCustomJarMods = false;

        ErrorReporting.recordInstancePlay(instance.getPackName(), instance.getVersion(), instance.getLoaderVersion(),
            2);

        int permGen = Optional.ofNullable(instance.launcher.permGen).orElse(App.settings.metaspace);
        String javaArguments = Optional.ofNullable(instance.launcher.javaArguments).orElse(App.settings.javaParameters);
        String javaPath = instance.getJavaPath();

        File jarMods = instance.getJarModsDirectory();
        File[] jarModFiles = jarMods.listFiles();
        if (jarMods.exists() && jarModFiles != null) {
            for (File file : jarModFiles) {
                hasCustomJarMods = true;
                cpb.append(file.getAbsolutePath());
                cpb.append(File.pathSeparator);
            }
        }

        Map<String, Library> dedupedLibraries = new LinkedHashMap<>();
        instance.libraries.stream().filter(
                library -> library.shouldInstall() && library.downloads.artifact != null && !library.hasNativeForOS())
            .filter(library -> library.downloads.artifact != null && library.downloads.artifact.path != null)
            .map(l -> LWJGLManager.shouldReplaceLWJGL3(instance)
                ? LWJGLManager.getReplacementLWJGL3Library(instance, l)
                : l)
            .forEach(library -> {
                try {
                    Pair<String, String> libraryName = Utils.convertMavenIdentifierToNameAndVersion(library.name);

                    if (dedupedLibraries.containsKey(libraryName.left())) {
                        Library existingLibrary = dedupedLibraries.get(libraryName.left());

                        String existingVersion = Utils.convertMavenIdentifierToNameAndVersion(existingLibrary.name)
                            .right();
                        String libraryVersion = Utils.convertMavenIdentifierToNameAndVersion(library.name).right();

                        if (Utils.compareVersions(libraryVersion, existingVersion) == 1) {
                            dedupedLibraries.put(libraryName.left(), library);
                        }
                    } else {
                        dedupedLibraries.put(libraryName.left(), library);
                    }
                } catch (Throwable throwable) {
                    LogManager.logStackTrace("Failed to dedupe library " + library.name + ". Adding regardless.",
                        throwable);

                    // worse case scenario, just add it to the list
                    dedupedLibraries.put(library.name, library);
                }
            });

        dedupedLibraries.values().stream()
            .forEach(library -> {
                String path = FileSystem.LIBRARIES.resolve(library.downloads.artifact.path).toFile()
                    .getAbsolutePath();

                if (cpb.indexOf(path) == -1) {
                    cpb.append(path);
                    cpb.append(File.pathSeparator);
                }
            });

        instance.libraries.stream().filter(Library::hasNativeForOS)
            .map(l -> LWJGLManager.shouldReplaceLWJGL3(instance)
                ? LWJGLManager.getReplacementLWJGL3Library(instance, l)
                : l)
            .forEach(library -> {
                com.blockforge.launcher.data.minecraft.Download download = library.getNativeDownloadForOS();

                cpb.append(FileSystem.LIBRARIES.resolve(download.path).toFile().getAbsolutePath());
                cpb.append(File.pathSeparator);
            });

        File binFolder = instance.getBinDirectory();
        File[] libraryFiles = binFolder.listFiles();
        if (binFolder.exists() && libraryFiles != null) {
            for (File file : libraryFiles) {
                if (!file.getName().equalsIgnoreCase("minecraft.jar")
                    && !file.getName().equalsIgnoreCase("modpack.jar")
                    && (file.getName().endsWith(".jar") || file.getName().endsWith(".zip"))) {
                    LogManager.info("Added in custom library " + file.getName());

                    cpb.append(file);
                    cpb.append(File.pathSeparator);
                }
            }
        }

        // add minecraft client jar last
        if (instance.usesCustomMinecraftJar()) {
            cpb.append(instance.getCustomMinecraftJar().getAbsolutePath());
        } else {
            cpb.append(instance.getMinecraftJar().getAbsolutePath());
        }

        if (instance.usesLegacyLaunch()) {
            Path legacyLaunchJarPath = FileSystem.LIBRARIES.resolve("launcher/legacy-launch.jar");

            try {
                if (!Files.exists(legacyLaunchJarPath) || Files.size(legacyLaunchJarPath) != 8368l) {
                    FileSystem.copyResourcesOutJar();
                }
            } catch (IOException e) {
                LogManager.logStackTrace("Failed to copy legacy-launch.jar to libraries folder", e);
            }

            cpb.append(File.pathSeparator);
            cpb.append(legacyLaunchJarPath.toAbsolutePath().toString());
        }

        List<String> arguments = new ArrayList<>();

        if (OS.isLinux() && App.settings.enableFeralGamemode && Utils.executableInPath("gamemoderun")) {
            arguments.add("gamemoderun");
        }

        String path = javaPath + File.separator + "bin" + File.separator + "java";
        if (OS.isWindows() && (Files.exists(Paths.get(path + "w")) || Files.exists(Paths.get(path + "w.exe")))) {
            path += "w";
        }
        int javaIndex = arguments.size();
        arguments.add(path);

        // automatic: what this game needs, as far as this computer can spare it; otherwise what the player set
        InstanceMemory.Decision memory = instance.takeMemoryDecision(javaPath);
        memory.warnings.forEach(LogManager::warn);
        int heapMb = memory.heapMb;
        LogManager.info(memory.automatic
            ? String.format(Locale.ENGLISH, "Memory: %d MB, picked automatically for %s (it would like %d MB; "
                + "this computer can spare up to %d MB)", heapMb, AutoMemory.describeGame(memory.game),
                memory.auto.idealMb, memory.auto.ceilingMb)
            : String.format(Locale.ENGLISH, "Memory: %d MB, as set in the settings", heapMb));
        // no -Xms: the game starts small and grows as needed, which matters on computers with little memory
        arguments.add("-Xmx" + heapMb + "M");
        if (!memory.automatic) {
            // the automatic choice has already looked at the free memory
            App.TASKPOOL.execute(() -> warnIfMemoryIsShort(heapMb));
        }

        // read once from the Java's release file instead of starting Java (twice) to ask
        int javaMajor = Java.parseJavaVersionNumber(Java.getVersionForJavaPath(new File(javaPath)));
        // if the version is unknown, assume Java 8 or newer since it's far more likely these days
        boolean useMetaspace = javaMajor <= 0 || javaMajor >= 8;

        if (OS.getMaximumRam() != 0 && permGen < instance.getPermGen()
            && (OS.getMaximumRam() / 8) < instance.getPermGen()) {
            if (useMetaspace) {
                arguments.add("-XX:MetaspaceSize=" + instance.getPermGen() + "M");
            } else {
                arguments.add("-XX:PermSize=" + instance.getPermGen() + "M");
            }
        } else {
            if (useMetaspace) {
                arguments.add("-XX:MetaspaceSize=" + permGen + "M");
            } else {
                arguments.add("-XX:PermSize=" + permGen + "M");
            }
        }

        arguments.add("-Duser.language=en");
        arguments.add("-Duser.country=US");

        if (hasCustomJarMods) {
            System.out.println("OH NOES! Avert your eyes!");
            arguments.add("-Dfml.ignorePatchDiscrepancies=true");
            arguments.add("-Dfml.ignoreInvalidMinecraftCertificates=true");
            System.out.println("Okay you can look again, you saw NOTHING!");
        }

        if (instance.logging != null && instance.logging.client != null) {
            LoggingClient loggingClient = instance.logging.client;

            Path loggingClientPath = FileSystem.RESOURCES_LOG_CONFIGS.resolve(loggingClient.file.id);

            if (Files.exists(loggingClientPath)) {
                arguments.add(loggingClient.getCompiledArgument());
            }
        }

        if (OS.isMac()) {
            arguments.add("-Dapple.laf.useScreenMenuBar=true");
            // no quotes: there's no shell in between, so they'd end up in the Dock as part of the name
            arguments.add("-Xdock:name=" + instance.getName());

            if (new File(instance.getAssetsDir(), "icons/minecraft.icns").exists()) {
                arguments.add(
                    "-Xdock:icon=" + new File(instance.getAssetsDir(), "icons/minecraft.icns").getAbsolutePath());
            }
        }

        if (JvmArguments.isCustom(javaArguments)) {
            // the person's own arguments are used exactly as they are
            arguments.addAll(JvmArguments.split(javaArguments));
        } else {
            // nothing set, or BlockForge's defaults: use the well known garbage collector settings, sized for the
            // memory this game gets
            JvmArguments.addMissing(arguments,
                JvmArguments.performanceFlags(heapMb, Runtime.getRuntime().availableProcessors()));
        }

        String classpath = cpb.toString();

        for (String argument : instance.arguments.jvmAsStringList()) {
            if (IGNORED_ARGUMENTS.contains(argument)) {
                continue;
            }

            arguments.add(replaceArgument(argument, instance, account, props, nativesDir, classpath, username));
        }

        if (OS.isWindows() && !arguments
            .contains("-XX:HeapDumpPath=MojangTricksIntelDriversForPerformance_javaw.exe_minecraft.exe.heapdump")) {
            arguments.add("-XX:HeapDumpPath=MojangTricksIntelDriversForPerformance_javaw.exe_minecraft.exe.heapdump");
        }

        // if there's no -Djava.library.path already, then add it (for older versions)
        if (!arguments.stream().anyMatch(arg -> arg.startsWith("-Djava.library.path="))) {
            arguments.add("-Djava.library.path=" + nativesDir);
        }

        // if lwjglNativesTempDir isn't null we need to pass the lwjgl librarypath
        if (lwjglNativesTempDir != null) {
            arguments.add("-Dorg.lwjgl.librarypath=" + lwjglNativesTempDir.toAbsolutePath().toString());
        }

        // if there's no classpath already, then add it (for older versions)
        if (!arguments.contains("-cp")) {
            arguments.add("-cp");
            arguments.add(cpb.toString());
        }

        int mainClassIndex = arguments.size();

        if (instance.usesLegacyLaunch()) {
            arguments.add("com.blockforge.launcher.mclauncher.legacy.LegacyMCLauncher");
            // Start or passed in arguments
            arguments.add(instance.getRootDirectory().getAbsolutePath()); // Path
            arguments.add(username); // Username
            arguments.add(account.getSessionToken()); // Session
            arguments.add(Constants.LAUNCHER_NAME + " - " + instance.getName()); // Frame title
            arguments.add(App.settings.windowWidth + ""); // Window Width
            arguments.add(App.settings.windowHeight + ""); // Window Height
            if (App.settings.maximiseMinecraft) {
                arguments.add("true"); // Maximised
            } else {
                arguments.add("false"); // Not Maximised
            }
        } else {
            arguments.add(instance.getMainClass());
        }

        if (!instance.usesLegacyLaunch()) {
            for (String argument : instance.arguments.gameAsStringList()) {
                if (IGNORED_ARGUMENTS.contains(argument)) {
                    continue;
                }

                arguments.add(replaceArgument(argument, instance, account, props, nativesDir, classpath, username));
            }

            if (App.settings.maximiseMinecraft) {
                arguments.add("--width=" + OS.getMaximumWindowWidth());
                arguments.add("--height=" + OS.getMaximumWindowHeight());
            } else {
                arguments.add("--width=" + App.settings.windowWidth);
                arguments.add("--height=" + App.settings.windowHeight);
            }
        }

        // Quick Play feature (with backward compatibility for older versions of
        // Minecraft)
        // "Continue" on Home asks for one world just this once, without changing the instance's setting
        QuickPlay quickPlay = instance.takeQuickPlayForLaunch();

        // Quick Play Multiplayer
        if (quickPlay.serverAddress != null && !quickPlay.serverAddress.isEmpty()) {
            String enteredServerAddress = quickPlay.serverAddress;
            if (instance.isQuickPlaySupported(QuickPlayOption.multiPlayer)) {
                // Minecraft 23w14a and newer versions
                arguments.addAll(
                    Arrays.asList(quickPlay.getSelectedQuickPlayOption().argumentRuleValue, enteredServerAddress));
            } else {
                // Minecraft 23w13a and older versions
                String[] parts = enteredServerAddress.contains(":") ? enteredServerAddress.split(":")
                    : new String[] { enteredServerAddress };
                String address = parts[0];
                String port = parts.length > 1 ? parts[1] : String.valueOf(Constants.MINECRAFT_DEFAULT_SERVER_PORT);
                arguments.addAll(Arrays.asList("--server", address));
                arguments.addAll(Arrays.asList("--port", port));
            }
        }

        // Quick Play Single Player
        if (quickPlay.worldName != null && !quickPlay.worldName.isEmpty()) {
            String selectedWorldSaveName = quickPlay.worldName;
            if (instance.isQuickPlaySupported(QuickPlayOption.singlePlayer)) {
                // Only work for Minecraft 23w14a and newer versions
                arguments.addAll(
                    Arrays.asList(quickPlay.getSelectedQuickPlayOption().argumentRuleValue, selectedWorldSaveName));
            }
        }

        // Quick Play Realm
        if (quickPlay.realmId != null && !quickPlay.realmId.isEmpty()) {
            String realmId = quickPlay.realmId;
            if (instance.isQuickPlaySupported(QuickPlayOption.realm)) {
                // Only work for Minecraft 23w14a and newer versions
                arguments.addAll(Arrays.asList(quickPlay.getSelectedQuickPlayOption().argumentRuleValue, realmId));
            }
        }

        // a class-data archive per game and Java makes later starts quicker; the person's own sharing options win
        mainClassIndex += ClassDataCache.apply(arguments, javaIndex, mainClassIndex, App.settings.classDataCache,
            javaMajor, javaPath, JvmArguments.split(javaArguments), instance.getSafeName());

        if (JvmArguments.needsArgumentFile(arguments, OS.isWindows(), javaMajor)) {
            moveJavaOptionsToArgumentFile(arguments, javaIndex, mainClassIndex, Paths.get(nativesDir));
        }

        return arguments;
    }

    /**
     * Windows can't start a program with a command line over 32767 characters, which big modpacks with hundreds of
     * libraries reach ("CreateProcess error=206"). Java 9 and newer can read their options from a file instead, so the
     * options (mostly the class path) go there. Only the options before the main class go in the file, so the account
     * details that follow it are never written to disk.
     */
    private static void moveJavaOptionsToArgumentFile(List<String> arguments, int javaIndex, int mainClassIndex,
        Path directory) {
        List<String> options = arguments.subList(javaIndex + 1, mainClassIndex);
        Path argumentFile = directory.resolve("java-options.txt");

        try {
            Files.createDirectories(directory);
            Files.write(argumentFile, JvmArguments.toArgumentFile(options).getBytes(StandardCharsets.UTF_8));
        } catch (IOException e) {
            LogManager.logStackTrace("Couldn't write the Java options file, trying the long command line", e);
            return;
        }

        LogManager.info("The command line is too long for Windows, so the Java options are passed in "
            + argumentFile.getFileName());
        options.clear();
        arguments.add(javaIndex + 1, "@" + argumentFile.toAbsolutePath());
    }

    private static String replaceArgument(String incomingArgument, Instance instance, MicrosoftAccount account,
        String props, String nativesDir, String classpath, String username) {
        String argument = incomingArgument;

        argument = argument.replace("${auth_player_name}", username);
        argument = argument.replace("${profile_name}", instance.getName());
        argument = argument.replace("${user_properties}", Optional.ofNullable(props).orElse("[]"));
        argument = argument.replace("${version_name}", instance.getMinecraftVersion());
        argument = argument.replace("${game_directory}", instance.getRootDirectory().getAbsolutePath());
        argument = argument.replace("${game_assets}", instance.getAssetsDir().getAbsolutePath());
        argument = argument.replace("${assets_root}", FileSystem.ASSETS.toAbsolutePath().toString());
        argument = argument.replace("${assets_index_name}", instance.getAssets());
        argument = argument.replace("${auth_uuid}", account.getRealUUID().toString());
        argument = argument.replace("${auth_access_token}", account.getAccessToken());
        argument = argument.replace("${version_type}", instance.type.getValue());
        argument = argument.replace("${launcher_name}", Constants.LAUNCHER_NAME);
        argument = argument.replace("${launcher_version}", Constants.VERSION.toStringForLogging());
        argument = argument.replace("${natives_directory}", nativesDir);
        argument = argument.replace("${user_type}", account.getUserType());
        argument = argument.replace("${auth_session}", account.getSessionToken());
        argument = argument.replace("${library_directory}", FileSystem.LIBRARIES.toAbsolutePath().toString());
        argument = argument.replace("${classpath}", classpath);
        argument = argument.replace("${classpath_separator}", File.pathSeparator);

        return argument;
    }

    private static String censorArguments(List<String> arguments, MicrosoftAccount account, String props,
        String username) {
        String argsString = arguments.toString();

        if (!LogManager.showDebug) {
            if (App.settings != null) {
                argsString = argsString.replace(FileSystem.BASE_DIR.toAbsolutePath().toString(), "USERSDIR");
            }

            argsString = argsString.replace(username, "REDACTED");
            argsString = argsString.replace(account.uuid, "REDACTED");
        }

        if (props != null) {
            argsString = argsString.replace(props, "REDACTED");
        }
        argsString = argsString.replace(account.getAccessToken(), "REDACTED");
        argsString = argsString.replace(account.getSessionToken(), "REDACTED");

        return argsString;
    }
}
