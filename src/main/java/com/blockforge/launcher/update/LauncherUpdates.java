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
package com.blockforge.launcher.update;

import java.io.IOException;
import java.io.InputStream;
import java.io.InterruptedIOException;
import java.io.OutputStream;
import java.nio.charset.StandardCharsets;
import java.nio.file.Files;
import java.nio.file.Path;
import java.nio.file.Paths;
import java.nio.file.StandardCopyOption;
import java.security.MessageDigest;
import java.security.NoSuchAlgorithmException;
import java.util.Locale;
import java.util.concurrent.Executors;
import java.util.concurrent.ScheduledExecutorService;
import java.util.concurrent.TimeUnit;
import java.util.concurrent.atomic.AtomicBoolean;
import java.util.function.Consumer;

import com.blockforge.launcher.App;
import com.blockforge.launcher.FileSystem;
import com.blockforge.launcher.Gsons;
import com.blockforge.launcher.Network;
import com.blockforge.launcher.constants.Constants;
import com.blockforge.launcher.managers.LogManager;
import com.blockforge.launcher.utils.OS;

import okhttp3.OkHttpClient;
import okhttp3.Request;
import okhttp3.Response;
import okhttp3.ResponseBody;

/**
 * Looks for newer BlockForge releases on GitHub, and downloads and starts the right installer for this computer.
 */
public final class LauncherUpdates {
    /** how often a running launcher looks again */
    private static final long CHECK_EVERY_HOURS = 6;
    /** wait a little after start so the check never competes with loading */
    private static final long FIRST_CHECK_DELAY_SECONDS = 8;

    private static ScheduledExecutorService scheduler;
    /** the release already offered while this launcher is open, so a later check doesn't nag again */
    private static volatile String offeredThisSession;

    private LauncherUpdates() {
    }

    /** What a check found. */
    public static final class Result {
        public final Release release;
        public final boolean newer;
        public final String error;

        Result(Release release, boolean newer, String error) {
            this.release = release;
            this.newer = newer;
            this.error = error;
        }
    }

    /** Receives download progress: bytes done so far and the total (or -1 when unknown). */
    public interface Progress {
        void update(long done, long total);
    }

    /**
     * Starts checking in the background: once shortly after start, then every few hours. {@code offer} is called (on
     * the checking thread) with each release that should be offered to the user.
     */
    public static synchronized void startBackgroundChecks(Consumer<Release> offer) {
        if (scheduler != null || App.noLauncherUpdate) {
            return;
        }
        scheduler = Executors.newSingleThreadScheduledExecutor(r -> {
            Thread thread = new Thread(r, "BlockForge update check");
            thread.setDaemon(true);
            return thread;
        });
        scheduler.scheduleWithFixedDelay(() -> {
            try {
                if (!App.settings.checkForLauncherUpdates) {
                    return;
                }
                Result result = check();
                if (shouldOffer(result)) {
                    offeredThisSession = result.release.tagName;
                    offer.accept(result.release);
                }
            } catch (Throwable t) {
                // a failed check must never take the scheduler down with it
                LogManager.debug("Update check failed: " + t.getMessage());
            }
        }, FIRST_CHECK_DELAY_SECONDS, TimeUnit.HOURS.toSeconds(CHECK_EVERY_HOURS), TimeUnit.SECONDS);
    }

    private static boolean shouldOffer(Result result) {
        if (result.error != null || !result.newer) {
            return false;
        }
        String tag = result.release.tagName;
        return !tag.equals(App.settings.skippedLauncherUpdate) && !tag.equals(offeredThisSession);
    }

    /** Asks GitHub for the latest release. Blocks, so call it off the UI thread. */
    public static Result check() {
        Request request = new Request.Builder().url(Constants.LATEST_RELEASE_API)
            .header("Accept", "application/vnd.github+json").header("X-GitHub-Api-Version", "2022-11-28").build();

        try (Response response = client().newCall(request).execute()) {
            if (response.code() == 404) {
                // the repository has no published release yet
                return new Result(null, false, null);
            }
            if (!response.isSuccessful() || response.body() == null) {
                return new Result(null, false, "GitHub answered " + response.code());
            }
            Release release = Gsons.DEFAULT.fromJson(response.body().string(), Release.class);
            if (release == null || release.tagName == null || release.draft || release.prerelease) {
                return new Result(release, false, null);
            }
            boolean newer = UpdatePlan.isNewer(release.tagName, currentVersion());
            LogManager.debug("Latest release on GitHub is " + release.tagName + (newer ? " (newer)" : ""));
            return new Result(release, newer, null);
        } catch (Exception e) {
            return new Result(null, false, e.getMessage() == null ? e.getClass().getSimpleName() : e.getMessage());
        }
    }

    public static int[] currentVersion() {
        return new int[] { Constants.VERSION.getReserved(), Constants.VERSION.getMajor(), Constants.VERSION.getMinor(),
                Constants.VERSION.getRevision() };
    }

    public static String currentVersionText() {
        return Constants.VERSION.getReserved() + "." + Constants.VERSION.getMajor() + "."
            + Constants.VERSION.getMinor() + "." + Constants.VERSION.getRevision();
    }

    /** How this copy was installed, which decides the file an update needs. */
    public static UpdatePlan.Kind installKind() {
        String osRelease = null;
        if (OS.isLinux()) {
            try {
                Path file = Paths.get("/etc/os-release");
                if (Files.isReadable(file)) {
                    osRelease = new String(Files.readAllBytes(file), StandardCharsets.UTF_8);
                }
            } catch (IOException ignored) {
                // no os-release means no package to offer, the release page still works
            }
        }

        String runningFrom = null;
        try {
            runningFrom = OS.getRunningProgramPath().toString();
        } catch (Exception ignored) {
            // treated like running from source
        }

        return UpdatePlan.detectKind(Boolean.getBoolean("blockforge.installed"), System.getProperty("os.name"),
            runningFrom, OS.isUsingFlatpak(), osRelease);
    }

    /**
     * Downloads the file into the launcher's temp folder, checking its size and checksum. A jar update is saved under
     * the running jar's own name because the swap step copies it over the old one by name.
     */
    public static Path download(Release.Asset asset, UpdatePlan.Kind kind, Progress progress, AtomicBoolean cancelled)
        throws IOException {
        Path folder = FileSystem.TEMP.resolve("update");
        Files.createDirectories(folder);
        String name = kind == UpdatePlan.Kind.JAR ? OS.getRunningProgramPath().getFileName().toString()
            : Paths.get(asset.name).getFileName().toString();
        Path target = folder.resolve(name);
        Path part = folder.resolve(name + ".part");

        Request request = new Request.Builder().url(asset.url).header("Accept", "application/octet-stream").build();
        MessageDigest sha256;
        try {
            sha256 = MessageDigest.getInstance("SHA-256");
        } catch (NoSuchAlgorithmException e) {
            throw new IOException(e);
        }

        long done = 0;
        try (Response response = client().newCall(request).execute()) {
            ResponseBody body = response.body();
            if (!response.isSuccessful() || body == null) {
                throw new IOException("Download failed, the server answered " + response.code());
            }
            long total = body.contentLength() > 0 ? body.contentLength() : asset.size;
            try (InputStream in = body.byteStream(); OutputStream out = Files.newOutputStream(part)) {
                byte[] buffer = new byte[64 * 1024];
                int read;
                long lastReport = 0;
                while ((read = in.read(buffer)) != -1) {
                    if (cancelled.get()) {
                        throw new InterruptedIOException("Download cancelled");
                    }
                    out.write(buffer, 0, read);
                    sha256.update(buffer, 0, read);
                    done += read;
                    if (done - lastReport >= 256 * 1024) {
                        lastReport = done;
                        progress.update(done, total);
                    }
                }
                progress.update(done, total);
            }
        } catch (IOException e) {
            Files.deleteIfExists(part);
            throw e;
        }

        if (asset.size > 0 && done != asset.size) {
            Files.deleteIfExists(part);
            throw new IOException("The download is incomplete (" + done + " of " + asset.size + " bytes)");
        }
        String expected = asset.sha256();
        if (expected != null) {
            String actual = hex(sha256.digest());
            if (!expected.equals(actual)) {
                Files.deleteIfExists(part);
                throw new IOException("The download is damaged (checksum mismatch)");
            }
        }

        Files.move(part, target, StandardCopyOption.REPLACE_EXISTING);
        return target;
    }

    /**
     * Starts the downloaded update. Returns true when the launcher is about to close for it (jar, Windows, macOS),
     * false when the system's software installer was opened and BlockForge stays open (Linux packages).
     */
    public static boolean install(Path file, UpdatePlan.Kind kind) throws IOException {
        LogManager.info("Installing launcher update from " + file);
        switch (kind) {
            case JAR:
                // swaps the jar and starts it again; exits this launcher
                App.launcher.runUpdate(OS.getRunningProgramPath().toString(), file.toAbsolutePath().toString());
                return true;
            case WINDOWS_EXE:
                // the setup program replaces this install (same upgrade id), so this copy has to close first
                new ProcessBuilder(file.toAbsolutePath().toString()).start();
                exitSoon();
                return true;
            case MAC_DMG:
                new ProcessBuilder("open", file.toAbsolutePath().toString()).start();
                exitSoon();
                return true;
            case DEB:
            case RPM:
                OS.openFile(file);
                return false;
            default:
                throw new IOException("This copy of BlockForge can't be updated automatically");
        }
    }

    /** The terminal command that installs a downloaded Linux package, shown in case no installer window opens. */
    public static String linuxInstallCommand(Path file, UpdatePlan.Kind kind) {
        String quoted = "'" + file.toAbsolutePath().toString().replace("'", "'\\''") + "'";
        return kind == UpdatePlan.Kind.DEB ? "sudo apt install " + quoted : "sudo dnf install " + quoted;
    }

    private static void exitSoon() {
        Thread exit = new Thread(() -> {
            try {
                Thread.sleep(800);
            } catch (InterruptedException ignored) {
                Thread.currentThread().interrupt();
            }
            System.exit(0);
        }, "BlockForge update exit");
        exit.setDaemon(false);
        exit.start();
    }

    private static OkHttpClient client() {
        OkHttpClient base = Network.CLIENT;
        if (base == null) {
            base = new OkHttpClient();
        }
        return base.newBuilder().readTimeout(60, TimeUnit.SECONDS).build();
    }

    static String hex(byte[] bytes) {
        StringBuilder sb = new StringBuilder(bytes.length * 2);
        for (byte b : bytes) {
            sb.append(String.format(Locale.ROOT, "%02x", b));
        }
        return sb.toString();
    }
}
