/*
 * BlockForge bridge: the one small jar that carries people who run the plain Java edition
 * (BlockForge-3.4.x.jar) over to the native BlockForge.
 *
 * Single file, no dependencies, Java 8 compatible. Build with build.sh or build.ps1.
 *
 * What it does when started (java -jar BlockForge-<ver>.jar [args]):
 *   1. works out the data folder the Java edition used for a jar run, so instances, accounts and settings
 *      are kept (the native build is started with BLOCKFORGE_DIR pointing at it);
 *   2. if the native build for this jar's version is not on disk yet, downloads it from the same GitHub
 *      release, checks its SHA-256 against the release's SHA256SUMS.txt, and stores it in <data>/bin/<ver>/;
 *   3. starts the native build and exits. Later runs skip steps 2 and work offline.
 *
 * It never deletes anything that belongs to the user: it only ever removes its own scratch folders under
 * <data>/bin/.tmp-*.
 *
 * The old in-app updater downloads this jar and starts "java -cp <jar> com.blockforge.launcher.Update
 * <old jar> <new jar> args...", so that class must exist here too (see Update at the bottom).
 */
package com.blockforge.launcher;

import java.awt.GraphicsEnvironment;
import java.io.BufferedReader;
import java.io.File;
import java.io.IOException;
import java.io.InputStream;
import java.io.InputStreamReader;
import java.io.OutputStream;
import java.net.HttpURLConnection;
import java.net.URL;
import java.nio.charset.StandardCharsets;
import java.nio.file.Files;
import java.nio.file.Path;
import java.nio.file.Paths;
import java.nio.file.StandardCopyOption;
import java.security.MessageDigest;
import java.util.ArrayList;
import java.util.Arrays;
import java.util.Collections;
import java.util.HashSet;
import java.util.LinkedHashMap;
import java.util.List;
import java.util.Locale;
import java.util.Map;
import java.util.Set;
import java.util.concurrent.TimeUnit;
import java.util.jar.JarFile;
import java.util.stream.Collectors;
import java.util.stream.Stream;
import java.util.zip.ZipEntry;
import java.util.zip.ZipFile;

public final class Bridge {
    static final String REPO = "riyadmondol2006/BlockForge";
    static final String LAUNCHER_NAME = "BlockForge";
    static final String RELEASES_PAGE = "https://github.com/" + REPO + "/releases/latest";
    static final String MARKER = ".complete";

    private Bridge() {
    }

    public static void main(String[] args) {
        int code;
        try {
            code = run(codeSourcePath(), null, args);
        } catch (Throwable t) {
            code = fail(null, null, "Something unexpected went wrong: " + describe(t), RELEASES_PAGE);
        }
        System.exit(code);
    }

    // ------------------------------------------------------------------------------------------------ main flow

    /**
     * @param jarPath  the jar this bridge is running from (its manifest tells the version)
     * @param dataHint where the portable data folder is when it is not next to {@code jarPath}, or null
     */
    static int run(Path jarPath, Path dataHint, String[] rawArgs) {
        Console console = new Console();
        Path data = null;
        try {
            System.setProperty("java.net.useSystemProxies", "true");
            Args args = Args.translate(rawArgs);
            data = resolveDataDir(jarPath, dataHint, args.workingDir);
            Path bin = data.resolve("bin");
            String version = manifestVersion(jarPath);

            Installed installed = version == null ? null : findInstalled(bin, version);
            String releasePage = RELEASES_PAGE;
            if (installed == null) {
                try {
                    console.open("Updating BlockForge, one moment...");
                    Release release = null;
                    try {
                        release = fetchRelease(version);
                        releasePage = release.htmlUrl != null ? release.htmlUrl : releasePage;
                        try {
                            installed = install(release, bin, console);
                        } catch (IOException e) {
                            if (release.apiProblem != null) {
                                throw new IOException(describe(e) + "\n(GitHub's release details could not be read: "
                                    + release.apiProblem + ")", e);
                            }
                            throw e;
                        }
                    } catch (IOException e) {
                        // offline or the release is not reachable: an already downloaded build still works
                        Installed other = newestInstalled(bin);
                        if (other == null) {
                            throw e;
                        }
                        console.note("Could not reach GitHub (" + describe(e) + "). Starting the BlockForge already on this computer.");
                        installed = other;
                    }
                    if (release != null && version != null && installed != null && !installed.version.equals(version)) {
                        writeLink(bin, version, installed.version);
                    }
                } finally {
                    console.close();
                }
            }
            return launch(installed, data, args, console, releasePage);
        } catch (Throwable t) {
            return fail(console, data, describe(t), RELEASES_PAGE);
        }
    }

    // ------------------------------------------------------------------------------------------------ data folder

    /** The folder the Java edition used, so the native build sees the same instances, accounts and settings. */
    static Path resolveDataDir(Path jarPath, Path dataHint, String workingDirArg) {
        if (workingDirArg != null && !workingDirArg.isEmpty()) {
            return Paths.get(workingDirArg).toAbsolutePath().normalize();
        }
        String env = System.getenv("BLOCKFORGE_DIR");
        if (env != null && !env.isEmpty()) {
            return Paths.get(env).toAbsolutePath().normalize();
        }
        if (Boolean.getBoolean("blockforge.installed")) {
            return userDataDirectory(System.getProperty("os.name", ""), System.getProperty("user.home"),
                System.getenv("APPDATA"), System.getenv("XDG_DATA_HOME"));
        }
        Path jarDir = dataHint != null ? dataHint
            : (jarPath != null && jarPath.toAbsolutePath().getParent() != null ? jarPath.toAbsolutePath().getParent()
                : Paths.get("").toAbsolutePath());
        Path cwd = Paths.get(System.getProperty("user.dir")).toAbsolutePath();
        // The Java edition used the jar's folder on Linux and the working directory elsewhere.
        List<Path> candidates = new ArrayList<>();
        if (isLinux()) {
            candidates.add(jarDir);
        } else {
            candidates.add(cwd);
            if (!jarDir.equals(cwd)) {
                candidates.add(jarDir);
            }
        }
        for (Path candidate : candidates) {
            if (looksLikeData(candidate)) {
                return candidate;
            }
        }
        // a fresh start: the Java edition's own choice, unless that folder can't be written to
        for (Path candidate : candidates) {
            if (Files.isWritable(candidate)) {
                return candidate;
            }
        }
        return candidates.get(0);
    }

    static boolean looksLikeData(Path dir) {
        return Files.isRegularFile(dir.resolve("configs").resolve("accounts.json"))
            || Files.isRegularFile(dir.resolve("configs").resolve(LAUNCHER_NAME + ".json"))
            || Files.isRegularFile(dir.resolve("configs").resolve(LAUNCHER_NAME + ".conf"))
            || Files.isDirectory(dir.resolve("instances"));
    }

    /** Same rule as the Java edition's FileSystem.userDataDirectory. */
    static Path userDataDirectory(String os, String home, String appData, String xdgDataHome) {
        String lower = os.toLowerCase(Locale.ROOT);
        if (lower.contains("win")) {
            Path base = appData != null && !appData.isEmpty() ? Paths.get(appData)
                : Paths.get(home, "AppData", "Roaming");
            return base.resolve(LAUNCHER_NAME);
        }
        if (lower.contains("mac")) {
            return Paths.get(home, "Library", "Application Support", LAUNCHER_NAME);
        }
        Path base = xdgDataHome != null && !xdgDataHome.isEmpty() ? Paths.get(xdgDataHome)
            : Paths.get(home, ".local", "share");
        return base.resolve(LAUNCHER_NAME.toLowerCase(Locale.ROOT));
    }

    // ------------------------------------------------------------------------------------------------ arguments

    /** The Java edition's command line, turned into what the native build understands. */
    static final class Args {
        String workingDir;
        final List<String> global = new ArrayList<>();
        final List<String> rest = new ArrayList<>();

        List<String> all() {
            List<String> out = new ArrayList<>(global);
            out.addAll(rest);
            return out;
        }

        private static final Set<String> JAVA_VALUE = new HashSet<>(Arrays.asList("install-method", "working-dir",
            "base-launcher-domain", "base-cdn-domain", "base-cdn-path", "launch", "launch-server", "proxy-type",
            "proxy-host", "proxy-port", "config-override", "debug-level"));
        private static final Set<String> JAVA_FLAG = new HashSet<>(Arrays.asList("updated", "updatedBundledJre",
            "skip-setup-dialog", "skip-tray-integration", "disable-analytics", "enable-error-reporting",
            "allow-all-ssl-certs", "no-launcher-update", "no-console", "close-launcher", "debug", "server-gui"));

        static Args translate(String[] raw) {
            Args a = new Args();
            for (int i = 0; i < raw.length; i++) {
                String tok = raw[i];
                if (tok.equals("-v") || tok.equals("--version")) {
                    a.rest.add("--version");
                    continue;
                }
                if (tok.equals("-?")) {
                    a.rest.add("--help");
                    continue;
                }
                if (!tok.startsWith("--")) {
                    a.rest.add(tok);
                    continue;
                }
                int eq = tok.indexOf('=');
                String name = eq < 0 ? tok.substring(2) : tok.substring(2, eq);
                String value = eq < 0 ? null : tok.substring(eq + 1);
                if (JAVA_VALUE.contains(name)) {
                    if (value == null && i + 1 < raw.length) {
                        value = raw[++i];
                    }
                    if (value == null) {
                        continue;
                    }
                    if (name.equals("working-dir")) {
                        a.workingDir = value;
                    } else if (name.equals("launch")) {
                        a.rest.add("launch");
                        a.rest.add(value);
                    } else if (name.equals("debug-level")) {
                        a.global.add("--debug-level");
                        a.global.add(value);
                    }
                    // the rest are Java edition settings the native build does not take on the command line
                } else if (JAVA_FLAG.contains(name)) {
                    if (name.equals("debug") && !"false".equalsIgnoreCase(value) && !a.global.contains("--debug")) {
                        a.global.add("--debug");
                    }
                } else {
                    a.rest.add(tok);
                }
            }
            return a;
        }
    }

    // ------------------------------------------------------------------------------------------------ platform

    static String osName() {
        String os = System.getProperty("os.name", "").toLowerCase(Locale.ROOT);
        return os.startsWith("windows") ? "windows" : os.contains("mac") || os.contains("darwin") ? "mac" : "linux";
    }

    static boolean isLinux() {
        return osName().equals("linux");
    }

    /** "arm64" or "x64". On a Mac this looks at the hardware, since a Java running under Rosetta says x86_64. */
    static String archName() {
        String arch = System.getProperty("os.arch", "").toLowerCase(Locale.ROOT);
        String os = osName();
        if (os.equals("windows")) {
            String w = System.getenv("PROCESSOR_ARCHITEW6432");
            String p = System.getenv("PROCESSOR_ARCHITECTURE");
            String real = (w != null && !w.isEmpty() ? w : p == null ? "" : p).toLowerCase(Locale.ROOT);
            if (real.contains("arm64")) {
                return "arm64";
            }
            if (real.contains("64")) {
                return "x64";
            }
        }
        if (os.equals("mac") && !arch.contains("aarch64") && !arch.contains("arm64")) {
            try {
                String out = exec(Arrays.asList("sysctl", "-n", "hw.optional.arm64"), 5).trim();
                if (out.endsWith("1")) {
                    return "arm64";
                }
            } catch (Exception ignored) {
                // not on a Mac after all, or no sysctl
            }
        }
        return arch.contains("aarch64") || arch.contains("arm64") ? "arm64" : "x64";
    }

    // ------------------------------------------------------------------------------------------------ release

    static final class Asset {
        String name;
        String url;
        long size;
        String digest;
    }

    static final class Release {
        String tag;
        String htmlUrl;
        /** set when GitHub's API could not be used and the file names were worked out instead */
        String apiProblem;
        final List<Asset> assets = new ArrayList<>();
    }

    static String apiBase() {
        String v = System.getProperty("blockforge.bridge.api");
        if (v == null || v.isEmpty()) {
            v = System.getenv("BLOCKFORGE_BRIDGE_API");
        }
        if (v == null || v.isEmpty()) {
            v = "https://api.github.com";
        }
        return v.endsWith("/") ? v.substring(0, v.length() - 1) : v;
    }

    /** Test only: where release files are fetched from when the API can't be used. */
    static String downloadsBase() {
        String v = System.getenv("BLOCKFORGE_BRIDGE_DOWNLOADS");
        if (v == null || v.isEmpty()) {
            v = "https://github.com/" + REPO + "/releases/download";
        }
        return v.endsWith("/") ? v.substring(0, v.length() - 1) : v;
    }

    @SuppressWarnings("unchecked")
    static Release fetchRelease(String version) throws IOException {
        String base = apiBase() + "/repos/" + REPO + "/releases/";
        List<String> urls = new ArrayList<>();
        if (version != null) {
            urls.add(base + "tags/v" + version);
            urls.add(base + "tags/" + version);
        }
        urls.add(base + "latest");

        IOException last = null;
        for (String url : urls) {
            try {
                String text = httpText(url);
                Object parsed = Json.parse(text);
                if (!(parsed instanceof Map)) {
                    throw new IOException("GitHub sent something unexpected");
                }
                Map<String, Object> m = (Map<String, Object>) parsed;
                Release r = new Release();
                r.tag = str(m.get("tag_name"));
                r.htmlUrl = str(m.get("html_url"));
                Object assets = m.get("assets");
                if (assets instanceof List) {
                    for (Object o : (List<Object>) assets) {
                        if (!(o instanceof Map)) {
                            continue;
                        }
                        Map<String, Object> am = (Map<String, Object>) o;
                        Asset a = new Asset();
                        a.name = str(am.get("name"));
                        a.url = str(am.get("browser_download_url"));
                        a.digest = str(am.get("digest"));
                        Object size = am.get("size");
                        a.size = size instanceof Number ? ((Number) size).longValue() : 0;
                        if (a.name != null && a.url != null) {
                            r.assets.add(a);
                        }
                    }
                }
                if (r.tag == null) {
                    throw new IOException("GitHub sent a release without a version");
                }
                return r;
            } catch (HttpError e) {
                last = e;
                if (e.code != 404) {
                    break;
                }
            } catch (IOException e) {
                last = e;
                break;
            }
        }
        if (version != null) {
            // GitHub's API can be rate limited or blocked while plain downloads still work
            Release r = new Release();
            r.tag = "v" + version;
            r.htmlUrl = "https://github.com/" + REPO + "/releases/tag/v" + version;
            r.apiProblem = last != null ? describe(last) : "no answer";
            for (String name : expectedAssetNames(version)) {
                Asset a = new Asset();
                a.name = name;
                a.url = downloadsBase() + "/v" + version + "/" + name;
                r.assets.add(a);
            }
            return r;
        }
        throw last != null ? last : new IOException("No release found");
    }

    static List<String> expectedAssetNames(String version) {
        String p = LAUNCHER_NAME + "-" + version + "-";
        List<String> n = new ArrayList<>();
        n.add("SHA256SUMS.txt");
        n.add("SHA256SUMS");
        String os = osName();
        boolean arm = archName().equals("arm64");
        if (os.equals("linux")) {
            n.add(p + (arm ? "linux-aarch64" : "linux-x86_64") + ".AppImage");
        } else if (os.equals("windows")) {
            n.add(p + (arm ? "windows-arm64" : "windows-x64") + "-portable.zip");
        } else {
            n.add(p + (arm ? "macos-arm64" : "macos-x64") + ".app.tar.gz");
            n.add(p + (arm ? "macos-arm64" : "macos-x64") + ".dmg");
        }
        return n;
    }

    private static String str(Object o) {
        return o instanceof String ? (String) o : null;
    }

    /** True when {@code name} (lower case) contains {@code token} as a whole word between separators. */
    static boolean hasToken(String name, String token) {
        int from = 0;
        while (true) {
            int i = name.indexOf(token, from);
            if (i < 0) {
                return false;
            }
            int end = i + token.length();
            boolean before = i == 0 || !Character.isLetterOrDigit(name.charAt(i - 1));
            boolean after = end >= name.length() || !Character.isLetterOrDigit(name.charAt(end));
            if (before && after) {
                return true;
            }
            from = i + 1;
        }
    }

    static boolean hasAny(String name, String... tokens) {
        for (String t : tokens) {
            if (hasToken(name, t)) {
                return true;
            }
        }
        return false;
    }

    /** Which release file is the native build for this computer. Returns null when there is none. */
    static Asset pickNative(Release release, String os, String arch) {
        String[][] wanted;
        if (os.equals("linux")) {
            wanted = new String[][] { { "linux", ".appimage" } };
        } else if (os.equals("windows")) {
            wanted = new String[][] { { "windows", "-portable.zip" }, { "win", "-portable.zip" } };
        } else {
            wanted = new String[][] { { "macos", ".app.tar.gz" }, { "macos", ".dmg" } };
        }
        boolean arm = arch.equals("arm64");
        String[] mine = arm ? new String[] { "arm64", "aarch64" } : new String[] { "x64", "x86_64", "amd64", "intel" };
        String[] other = arm ? new String[] { "x64", "x86_64", "amd64", "intel" } : new String[] { "arm64", "aarch64" };

        for (String[] w : wanted) {
            Asset neutral = null;
            Asset foreign = null;
            for (Asset a : release.assets) {
                String n = a.name.toLowerCase(Locale.ROOT);
                if (!n.endsWith(w[1]) || !hasToken(n, w[0])) {
                    continue;
                }
                if (hasAny(n, mine) || hasToken(n, "universal")) {
                    return a;
                }
                if (hasAny(n, other)) {
                    if (foreign == null) {
                        foreign = a;
                    }
                } else if (neutral == null) {
                    neutral = a;
                }
            }
            if (neutral != null) {
                return neutral;
            }
            // Windows on ARM runs the x64 build through Windows' own emulation
            if (os.equals("windows") && arm && foreign != null) {
                return foreign;
            }
        }
        return null;
    }

    static Asset findChecksums(Release release) {
        for (String name : new String[] { "SHA256SUMS.txt", "SHA256SUMS" }) {
            for (Asset a : release.assets) {
                if (a.name.equalsIgnoreCase(name)) {
                    return a;
                }
            }
        }
        return null;
    }

    /** Reads "<sha256>  <name>" lines and returns the hash for {@code name}, or null. */
    static String checksumFor(String sums, String name) {
        for (String line : sums.split("\\r?\\n")) {
            String t = line.trim();
            if (t.length() < 66) {
                continue;
            }
            String hash = t.substring(0, 64);
            if (!hash.matches("[0-9a-fA-F]{64}")) {
                continue;
            }
            String file = t.substring(64).trim();
            if (file.startsWith("*")) {
                file = file.substring(1);
            }
            int slash = Math.max(file.lastIndexOf('/'), file.lastIndexOf('\\'));
            if (slash >= 0) {
                file = file.substring(slash + 1);
            }
            if (file.equals(name)) {
                return hash.toLowerCase(Locale.ROOT);
            }
        }
        return null;
    }

    // ------------------------------------------------------------------------------------------------ install

    static final class Installed {
        final String version;
        final Path exe;
        final boolean appImage;

        Installed(String version, Path exe) {
            this.version = version;
            this.exe = exe;
            this.appImage = exe.getFileName().toString().toLowerCase(Locale.ROOT).endsWith(".appimage");
        }
    }

    static String safeName(String s) {
        return s.replaceAll("[^0-9A-Za-z._-]", "_");
    }

    static Installed readInstalled(Path bin, String version) {
        Path dir = bin.resolve(safeName(version));
        Path marker = dir.resolve(MARKER);
        if (!Files.isRegularFile(marker)) {
            return null;
        }
        try {
            for (String line : Files.readAllLines(marker, StandardCharsets.UTF_8)) {
                if (line.startsWith("exe=")) {
                    Path exe = dir.resolve(line.substring(4)).normalize();
                    if (exe.startsWith(dir) && Files.isRegularFile(exe)) {
                        return new Installed(version, exe);
                    }
                }
            }
        } catch (IOException ignored) {
            // treated as not installed
        }
        return null;
    }

    static Installed findInstalled(Path bin, String version) {
        Installed direct = readInstalled(bin, version);
        if (direct != null) {
            return direct;
        }
        Path link = bin.resolve(safeName(version) + ".link");
        try {
            if (Files.isRegularFile(link)) {
                String target = new String(Files.readAllBytes(link), StandardCharsets.UTF_8).trim();
                if (!target.isEmpty()) {
                    return readInstalled(bin, target);
                }
            }
        } catch (IOException ignored) {
            // fall through
        }
        return null;
    }

    static void writeLink(Path bin, String from, String to) {
        try {
            Files.write(bin.resolve(safeName(from) + ".link"), to.getBytes(StandardCharsets.UTF_8));
        } catch (IOException ignored) {
            // only an optimisation for the next start
        }
    }

    static Installed newestInstalled(Path bin) {
        if (!Files.isDirectory(bin)) {
            return null;
        }
        List<String> names = new ArrayList<>();
        try (Stream<Path> s = Files.list(bin)) {
            for (Path p : (Iterable<Path>) s::iterator) {
                String n = p.getFileName().toString();
                if (Files.isDirectory(p) && !n.startsWith(".")) {
                    names.add(n);
                }
            }
        } catch (IOException e) {
            return null;
        }
        Collections.sort(names, (a, b) -> compareVersions(b, a));
        for (String n : names) {
            Installed i = readInstalled(bin, n);
            if (i != null) {
                return i;
            }
        }
        return null;
    }

    static int compareVersions(String a, String b) {
        String[] x = a.split("[^0-9]+");
        String[] y = b.split("[^0-9]+");
        for (int i = 0; i < Math.max(x.length, y.length); i++) {
            long p = i < x.length && !x[i].isEmpty() ? parseLong(x[i]) : 0;
            long q = i < y.length && !y[i].isEmpty() ? parseLong(y[i]) : 0;
            if (p != q) {
                return p < q ? -1 : 1;
            }
        }
        return 0;
    }

    private static long parseLong(String s) {
        try {
            return Long.parseLong(s);
        } catch (NumberFormatException e) {
            return 0;
        }
    }

    static Installed install(Release release, Path bin, Console console) throws IOException {
        String os = osName();
        String arch = archName();
        Asset asset = pickNative(release, os, arch);
        if (asset == null) {
            throw new IOException("This release has no BlockForge build for " + os + " " + arch + ".");
        }
        String version = release.tag.startsWith("v") || release.tag.startsWith("V") ? release.tag.substring(1)
            : release.tag;
        Installed already = readInstalled(bin, version);
        if (already != null) {
            return already;
        }

        // the expected checksum: SHA256SUMS.txt (or SHA256SUMS) first, GitHub's own digest as a second opinion
        String expected = null;
        Asset sums = findChecksums(release);
        if (sums != null) {
            expected = checksumFor(httpText(sums.url), asset.name);
            if (expected == null) {
                throw new IOException(sums.name + " has no entry for " + asset.name + ", so it cannot be verified.");
            }
        }
        if (asset.digest != null && asset.digest.toLowerCase(Locale.ROOT).startsWith("sha256:")) {
            String api = asset.digest.substring(7).trim().toLowerCase(Locale.ROOT);
            if (expected != null && !expected.equals(api)) {
                throw new IOException("The release's checksum file and GitHub disagree about " + asset.name
                    + ", so it was not used.");
            }
            if (expected == null) {
                expected = api;
            }
        }
        if (expected == null) {
            throw new IOException("The release has no SHA256SUMS.txt, so " + asset.name + " cannot be verified.");
        }

        Files.createDirectories(bin);
        Path tmp = Files.createTempDirectory(bin, ".tmp-" + safeName(version) + "-");
        try {
            Path file = tmp.resolve(safeName(asset.name));
            console.note("Downloading " + asset.name + " ...");
            String actual = download(asset.url, file, asset.size, console);
            if (!actual.equals(expected)) {
                throw new IOException("The downloaded " + asset.name + " does not match its checksum (expected "
                    + expected.substring(0, 12) + "..., got " + actual.substring(0, 12)
                    + "...). It was not run and has been discarded.");
            }
            console.note("Checksum verified.");

            Path stage = tmp.resolve("stage");
            Files.createDirectories(stage);
            String exeRel = unpack(file, asset.name, stage, console);

            List<String> marker = new ArrayList<>();
            marker.add("exe=" + exeRel);
            marker.add("asset=" + asset.name);
            marker.add("sha256=" + actual);
            marker.add("tag=" + release.tag);
            Files.write(stage.resolve(MARKER), marker, StandardCharsets.UTF_8);

            Path target = bin.resolve(safeName(version));
            if (Files.exists(target)) {
                Installed raced = readInstalled(bin, version);
                if (raced != null) {
                    return raced;
                }
                // an unfinished folder from an earlier try: keep it, just move it aside
                Files.move(target, bin.resolve(safeName(version) + ".old-" + System.currentTimeMillis()));
            }
            Files.move(stage, target);
            Installed done = readInstalled(bin, version);
            if (done == null) {
                throw new IOException("The download was unpacked but the program was not found.");
            }
            return done;
        } finally {
            deleteScratch(tmp, bin);
        }
    }

    /** Puts the downloaded file into {@code stage} ready to run. Returns the program's path inside stage. */
    static String unpack(Path file, String assetName, Path stage, Console console) throws IOException {
        String lower = assetName.toLowerCase(Locale.ROOT);
        if (lower.endsWith(".appimage")) {
            Path dest = stage.resolve(safeName(assetName));
            Files.move(file, dest);
            makeExecutable(dest);
            return dest.getFileName().toString();
        }
        if (lower.endsWith(".zip")) {
            unzip(file, stage);
            Path exe = find(stage, 3, p -> p.getFileName().toString().equalsIgnoreCase("blockforge.exe")
                || (osName().equals("windows") ? false : p.getFileName().toString().equalsIgnoreCase("blockforge")));
            if (exe == null) {
                throw new IOException("blockforge.exe was not in " + assetName);
            }
            makeExecutable(exe);
            return stage.relativize(exe).toString();
        }
        if (lower.endsWith(".tar.gz")) {
            String out = exec(Arrays.asList("tar", "-xzf", file.toString(), "-C", stage.toString()), 600);
            if (out == null) {
                throw new IOException("Could not unpack " + assetName);
            }
            return macExe(stage);
        }
        if (lower.endsWith(".dmg")) {
            Path mount = Files.createDirectory(file.resolveSibling("mnt"));
            boolean attached = false;
            try {
                exec(Arrays.asList("hdiutil", "attach", "-nobrowse", "-readonly", "-noverify", "-noautoopen",
                    "-mountpoint", mount.toString(), file.toString()), 300);
                attached = true;
                Path app = find(mount, 1, p -> p.getFileName().toString().endsWith(".app") && Files.isDirectory(p));
                if (app == null) {
                    throw new IOException("There is no app inside " + assetName);
                }
                exec(Arrays.asList("ditto", app.toString(), stage.resolve(app.getFileName().toString()).toString()),
                    600);
            } finally {
                if (attached) {
                    try {
                        exec(Arrays.asList("hdiutil", "detach", mount.toString()), 60);
                    } catch (Exception e) {
                        try {
                            exec(Arrays.asList("hdiutil", "detach", "-force", mount.toString()), 60);
                        } catch (Exception ignored) {
                            // leaving it mounted is harmless
                        }
                    }
                }
            }
            return macExe(stage);
        }
        throw new IOException("Don't know how to unpack " + assetName);
    }

    /** Finds Contents/MacOS/<program> in the app under {@code stage}, and lets macOS open it (no quarantine). */
    static String macExe(Path stage) throws IOException {
        Path app = find(stage, 1, p -> p.getFileName().toString().endsWith(".app") && Files.isDirectory(p));
        if (app == null) {
            throw new IOException("There is no app in the download.");
        }
        try {
            exec(Arrays.asList("xattr", "-dr", "com.apple.quarantine", app.toString()), 120);
        } catch (Exception ignored) {
            // no quarantine flag, or no xattr: nothing to clear
        }
        Path macos = app.resolve("Contents").resolve("MacOS");
        Path exe = null;
        if (Files.isDirectory(macos)) {
            try (Stream<Path> s = Files.list(macos)) {
                List<Path> files = s.filter(Files::isRegularFile).collect(Collectors.toList());
                for (Path f : files) {
                    if (f.getFileName().toString().equalsIgnoreCase("blockforge")) {
                        exe = f;
                    }
                }
                if (exe == null && !files.isEmpty()) {
                    exe = files.get(0);
                }
            }
        }
        if (exe == null) {
            throw new IOException("The app has no program inside it.");
        }
        makeExecutable(exe);
        return stage.relativize(exe).toString();
    }

    static void unzip(Path zip, Path dest) throws IOException {
        Path root = dest.toAbsolutePath().normalize();
        try (ZipFile zf = new ZipFile(zip.toFile())) {
            java.util.Enumeration<? extends ZipEntry> en = zf.entries();
            while (en.hasMoreElements()) {
                ZipEntry e = en.nextElement();
                Path out = root.resolve(e.getName()).normalize();
                if (!out.startsWith(root)) {
                    throw new IOException("The zip holds a file outside its folder (" + e.getName() + "); not unpacked.");
                }
                if (e.isDirectory()) {
                    Files.createDirectories(out);
                    continue;
                }
                Files.createDirectories(out.getParent());
                try (InputStream in = zf.getInputStream(e)) {
                    Files.copy(in, out, StandardCopyOption.REPLACE_EXISTING);
                }
            }
        }
    }

    static Path find(Path root, int depth, java.util.function.Predicate<Path> test) throws IOException {
        try (Stream<Path> s = Files.walk(root, depth)) {
            List<Path> hits = s.filter(p -> !p.equals(root)).filter(test).sorted().collect(Collectors.toList());
            return hits.isEmpty() ? null : hits.get(0);
        }
    }

    static void makeExecutable(Path p) {
        File f = p.toFile();
        if (!f.setExecutable(true, false)) {
            f.setExecutable(true);
        }
    }

    /** Removes our own scratch folder, and nothing else: the path must be a ".tmp-" folder directly under bin. */
    static void deleteScratch(Path dir, Path bin) {
        try {
            if (dir == null || !dir.getParent().equals(bin) || !dir.getFileName().toString().startsWith(".tmp-")) {
                return;
            }
            Files.walkFileTree(dir, new java.nio.file.SimpleFileVisitor<Path>() {
                @Override
                public java.nio.file.FileVisitResult visitFile(Path f, java.nio.file.attribute.BasicFileAttributes a)
                    throws IOException {
                    Files.deleteIfExists(f);
                    return java.nio.file.FileVisitResult.CONTINUE;
                }

                @Override
                public java.nio.file.FileVisitResult postVisitDirectory(Path d, IOException e) throws IOException {
                    Files.deleteIfExists(d);
                    return java.nio.file.FileVisitResult.CONTINUE;
                }
            });
        } catch (Exception ignored) {
            // a leftover scratch folder is harmless
        }
    }

    // ------------------------------------------------------------------------------------------------ launching

    static boolean fuseAvailable() {
        String forced = System.getenv("BLOCKFORGE_BRIDGE_FUSE");
        if (forced != null && !forced.isEmpty()) {
            return forced.equals("1");
        }
        File dev = new File("/dev/fuse");
        if (!dev.exists() || !dev.canRead() || !dev.canWrite()) {
            return false;
        }
        String[] dirs = { "/usr/lib64", "/usr/lib", "/lib64", "/lib", "/usr/lib/x86_64-linux-gnu",
            "/lib/x86_64-linux-gnu", "/usr/lib/aarch64-linux-gnu", "/lib/aarch64-linux-gnu", "/usr/local/lib" };
        for (String d : dirs) {
            File[] hits = new File(d).listFiles((dir, name) -> name.startsWith("libfuse.so.2"));
            if (hits != null && hits.length > 0) {
                return true;
            }
        }
        return false;
    }

    static int launch(Installed in, Path data, Args args, Console console, String releasePage) throws Exception {
        boolean wait = "1".equals(System.getenv("BLOCKFORGE_BRIDGE_WAIT"));
        boolean extract = in.appImage && isLinux() && !fuseAvailable();
        Path log = data.resolve("bin").resolve("launch.log");
        for (int attempt = 0; attempt < 2; attempt++) {
            List<String> cmd = new ArrayList<>();
            cmd.add(in.exe.toString());
            if (extract) {
                cmd.add("--appimage-extract-and-run");
            }
            cmd.addAll(args.all());

            ProcessBuilder pb = new ProcessBuilder(cmd);
            pb.directory(data.toFile());
            pb.environment().put("BLOCKFORGE_DIR", data.toString());
            boolean inherit = wait || System.console() != null;
            if (inherit) {
                pb.inheritIO();
            } else {
                Files.createDirectories(log.getParent());
                pb.redirectErrorStream(true);
                pb.redirectOutput(ProcessBuilder.Redirect.to(log.toFile()));
            }
            Process p = pb.start();
            if (wait) {
                return p.waitFor();
            }
            // give it a moment: a start that fails right away is reported instead of vanishing
            if (p.waitFor(2500, TimeUnit.MILLISECONDS)) {
                int code = p.exitValue();
                if (code == 0 || inherit && !args.all().isEmpty()) {
                    return code;
                }
                if (code != 0 && in.appImage && !extract && args.all().isEmpty()) {
                    extract = true;
                    continue;
                }
                if (inherit) {
                    return code;
                }
                return fail(console, data, "BlockForge stopped right after starting (exit code " + code + ").\n"
                    + tail(log), releasePage);
            }
            return 0;
        }
        return 1;
    }

    static String tail(Path log) {
        try {
            List<String> lines = Files.readAllLines(log, StandardCharsets.UTF_8);
            int from = Math.max(0, lines.size() - 15);
            return String.join("\n", lines.subList(from, lines.size()));
        } catch (Exception e) {
            return "";
        }
    }

    // ------------------------------------------------------------------------------------------------ errors

    static int fail(Console console, Path data, String message, String page) {
        String text = "BlockForge could not start.\n\n" + message + "\n\nNothing of yours was changed. "
            + "You can download BlockForge by hand from:\n" + page;
        System.err.println(text);
        if (data != null) {
            try {
                Path bin = data.resolve("bin");
                Files.createDirectories(bin);
                Files.write(bin.resolve("bridge-error.log"), text.getBytes(StandardCharsets.UTF_8));
            } catch (Exception ignored) {
                // the message above is enough
            }
        }
        if (console != null) {
            console.showError(text, page);
        } else {
            new Console().showError(text, page);
        }
        return 1;
    }

    static String describe(Throwable t) {
        String m = t.getMessage();
        if (t instanceof java.nio.file.AccessDeniedException) {
            return "BlockForge is not allowed to write to " + m
                + ". Move the jar into a folder of your own and start it from there.";
        }
        if (t instanceof java.net.UnknownHostException) {
            return "No internet connection (" + m + ").";
        }
        return m == null || m.isEmpty() ? t.getClass().getSimpleName() : m;
    }

    // ------------------------------------------------------------------------------------------------ network

    static final class HttpError extends IOException {
        private static final long serialVersionUID = 1L;
        final int code;

        HttpError(int code, String url) {
            super("The server answered " + code + " for " + url);
            this.code = code;
        }
    }

    static HttpURLConnection open(String url, String accept) throws IOException {
        String cur = url;
        for (int i = 0; i < 10; i++) {
            HttpURLConnection c = (HttpURLConnection) new URL(cur).openConnection();
            c.setInstanceFollowRedirects(false);
            c.setConnectTimeout(15000);
            c.setReadTimeout(60000);
            c.setRequestProperty("User-Agent", "BlockForge-Bridge");
            c.setRequestProperty("Accept", accept);
            int code = c.getResponseCode();
            if (code == 301 || code == 302 || code == 303 || code == 307 || code == 308) {
                String loc = c.getHeaderField("Location");
                c.disconnect();
                if (loc == null) {
                    throw new IOException("Bad redirect from " + cur);
                }
                cur = new URL(new URL(cur), loc).toString();
                continue;
            }
            if (code >= 400) {
                c.disconnect();
                throw new HttpError(code, url);
            }
            return c;
        }
        throw new IOException("Too many redirects for " + url);
    }

    static String httpText(String url) throws IOException {
        HttpURLConnection c = open(url, "application/vnd.github+json, text/plain, */*");
        try (InputStream in = c.getInputStream()) {
            java.io.ByteArrayOutputStream out = new java.io.ByteArrayOutputStream();
            byte[] buf = new byte[8192];
            int n;
            while ((n = in.read(buf)) != -1) {
                out.write(buf, 0, n);
            }
            return new String(out.toByteArray(), StandardCharsets.UTF_8);
        }
    }

    /** Downloads to {@code target} and returns the SHA-256 of what arrived. */
    static String download(String url, Path target, long expectedSize, Console console) throws IOException {
        MessageDigest md;
        try {
            md = MessageDigest.getInstance("SHA-256");
        } catch (java.security.NoSuchAlgorithmException e) {
            throw new IOException(e);
        }
        HttpURLConnection c = open(url, "application/octet-stream");
        long total = c.getContentLengthLong() > 0 ? c.getContentLengthLong() : expectedSize;
        long done = 0;
        try (InputStream in = c.getInputStream(); OutputStream out = Files.newOutputStream(target)) {
            byte[] buf = new byte[64 * 1024];
            int n;
            while ((n = in.read(buf)) != -1) {
                out.write(buf, 0, n);
                md.update(buf, 0, n);
                done += n;
                console.progress(done, total);
            }
        }
        if (total > 0 && done != total) {
            throw new IOException("The download is incomplete (" + done + " of " + total + " bytes).");
        }
        StringBuilder sb = new StringBuilder();
        for (byte b : md.digest()) {
            sb.append(String.format(Locale.ROOT, "%02x", b));
        }
        return sb.toString();
    }

    // ------------------------------------------------------------------------------------------------ helpers

    /** Runs a command and returns its output; throws when it fails or takes longer than {@code seconds}. */
    static String exec(List<String> cmd, int seconds) throws IOException {
        ProcessBuilder pb = new ProcessBuilder(cmd);
        pb.redirectErrorStream(true);
        Process p = pb.start();
        java.io.ByteArrayOutputStream out = new java.io.ByteArrayOutputStream();
        try (InputStream in = p.getInputStream()) {
            byte[] buf = new byte[4096];
            int n;
            while ((n = in.read(buf)) != -1) {
                out.write(buf, 0, n);
            }
        }
        try {
            if (!p.waitFor(seconds, TimeUnit.SECONDS)) {
                p.destroyForcibly();
                throw new IOException(cmd.get(0) + " took too long");
            }
        } catch (InterruptedException e) {
            Thread.currentThread().interrupt();
            throw new IOException(e);
        }
        String text = new String(out.toByteArray(), StandardCharsets.UTF_8);
        if (p.exitValue() != 0) {
            throw new IOException(cmd.get(0) + " failed: " + text.trim());
        }
        return text;
    }

    static Path codeSourcePath() throws Exception {
        return Paths.get(Bridge.class.getProtectionDomain().getCodeSource().getLocation().toURI())
            .toAbsolutePath();
    }

    static String manifestVersion(Path jar) {
        if (jar == null || !Files.isRegularFile(jar)) {
            return null;
        }
        try (JarFile jf = new JarFile(jar.toFile())) {
            if (jf.getManifest() == null) {
                return null;
            }
            String v = jf.getManifest().getMainAttributes().getValue("Implementation-Version");
            return v == null || v.trim().isEmpty() ? null : v.trim();
        } catch (IOException e) {
            return null;
        }
    }

    // ------------------------------------------------------------------------------------------------ old updater

    /**
     * The Java edition's updater runs this jar as "java -cp <this jar> com.blockforge.launcher.Update <old jar>
     * <this jar> args...": copy this jar over the old one, then start it again.
     */
    static void update(String[] a) {
        int code;
        try {
            Path launcher = Paths.get(a[0]).toAbsolutePath();
            Path temp = Paths.get(a[1]).toAbsolutePath();
            String[] rest = Arrays.copyOfRange(a, 2, a.length);
            Path runFrom = temp;
            Path newCopy = launcher.resolveSibling(launcher.getFileName() + ".new");
            // the old launcher is still closing; on Windows its jar stays locked for a moment
            for (int i = 0; i < 40; i++) {
                try {
                    Files.copy(temp, newCopy, StandardCopyOption.REPLACE_EXISTING);
                    try {
                        Files.move(newCopy, launcher, StandardCopyOption.REPLACE_EXISTING,
                            StandardCopyOption.ATOMIC_MOVE);
                    } catch (IOException atomicFailed) {
                        Files.move(newCopy, launcher, StandardCopyOption.REPLACE_EXISTING);
                    }
                    makeExecutable(launcher);
                    runFrom = launcher;
                    break;
                } catch (IOException e) {
                    Thread.sleep(500);
                }
            }
            Files.deleteIfExists(newCopy);
            code = run(runFrom, launcher.getParent(), rest);
        } catch (Throwable t) {
            code = fail(null, null, "The update could not be finished: " + describe(t), RELEASES_PAGE);
        }
        System.exit(code);
    }

    // ------------------------------------------------------------------------------------------------ progress UI

    /** "Updating BlockForge, one moment..." in a small window when there is a display, otherwise on stderr. */
    static final class Console {
        private Object window;
        private boolean tried;
        private int lastPct = -10;

        private boolean gui() {
            if ("1".equals(System.getenv("BLOCKFORGE_BRIDGE_NO_GUI"))) {
                return false;
            }
            try {
                if (GraphicsEnvironment.isHeadless()) {
                    return false;
                }
                if (isLinux() && isEmpty(System.getenv("DISPLAY")) && isEmpty(System.getenv("WAYLAND_DISPLAY"))) {
                    return false;
                }
                return true;
            } catch (Throwable t) {
                return false;
            }
        }

        private static boolean isEmpty(String s) {
            return s == null || s.isEmpty();
        }

        void open(String message) {
            System.err.println(message);
            if (tried) {
                return;
            }
            tried = true;
            if (gui()) {
                try {
                    window = Ui.open(message);
                } catch (Throwable t) {
                    window = null;
                }
            }
        }

        void note(String message) {
            System.err.println(message);
            if (window != null) {
                try {
                    ((Ui) window).text(message);
                } catch (Throwable ignored) {
                    // the console line above is enough
                }
            }
        }

        void progress(long done, long total) {
            if (total > 0) {
                int pct = (int) (done * 100 / total);
                if (pct >= lastPct + 10 || pct == 100 && lastPct != 100) {
                    lastPct = pct;
                    System.err.println("  " + pct + "%");
                }
            }
            if (window != null) {
                try {
                    ((Ui) window).progress(done, total);
                } catch (Throwable ignored) {
                    // cosmetic
                }
            }
        }

        void close() {
            if (window != null) {
                try {
                    ((Ui) window).close();
                } catch (Throwable ignored) {
                    // cosmetic
                }
                window = null;
            }
        }

        void showError(String text, String page) {
            close();
            if (!gui()) {
                return;
            }
            try {
                Ui.error(text, page);
            } catch (Throwable ignored) {
                // already printed on stderr
            }
        }
    }

    /** The Swing part, only loaded when a display exists. */
    static final class Ui {
        private javax.swing.JFrame frame;
        private javax.swing.JLabel label;
        private javax.swing.JProgressBar bar;

        static Ui open(String message) throws Exception {
            final Ui ui = new Ui();
            javax.swing.SwingUtilities.invokeAndWait(() -> {
                ui.frame = new javax.swing.JFrame("BlockForge");
                ui.frame.setDefaultCloseOperation(javax.swing.WindowConstants.DO_NOTHING_ON_CLOSE);
                ui.label = new javax.swing.JLabel(message);
                ui.bar = new javax.swing.JProgressBar(0, 100);
                ui.bar.setIndeterminate(true);
                javax.swing.JPanel panel = new javax.swing.JPanel(new java.awt.BorderLayout(0, 10));
                panel.setBorder(javax.swing.BorderFactory.createEmptyBorder(18, 22, 18, 22));
                panel.add(ui.label, java.awt.BorderLayout.NORTH);
                panel.add(ui.bar, java.awt.BorderLayout.CENTER);
                ui.frame.setContentPane(panel);
                ui.frame.setSize(380, 120);
                ui.frame.setLocationRelativeTo(null);
                ui.frame.setResizable(false);
                ui.frame.setVisible(true);
            });
            return ui;
        }

        void text(String message) {
            javax.swing.SwingUtilities.invokeLater(() -> label.setText(message));
        }

        void progress(long done, long total) {
            if (total <= 0) {
                return;
            }
            final int pct = (int) (done * 100 / total);
            javax.swing.SwingUtilities.invokeLater(() -> {
                bar.setIndeterminate(false);
                bar.setValue(pct);
            });
        }

        void close() throws Exception {
            javax.swing.SwingUtilities.invokeAndWait(() -> frame.dispose());
        }

        static void error(String text, String page) throws Exception {
            javax.swing.SwingUtilities.invokeAndWait(() -> {
                javax.swing.JTextArea area = new javax.swing.JTextArea(text, 10, 48);
                area.setEditable(false);
                area.setLineWrap(true);
                area.setWrapStyleWord(true);
                area.setOpaque(false);
                Object[] options = { "Open release page", "Close" };
                int choice = javax.swing.JOptionPane.showOptionDialog(null, area, "BlockForge",
                    javax.swing.JOptionPane.DEFAULT_OPTION, javax.swing.JOptionPane.ERROR_MESSAGE, null, options,
                    options[1]);
                if (choice == 0) {
                    try {
                        java.awt.Desktop.getDesktop().browse(new java.net.URI(page));
                    } catch (Throwable ignored) {
                        // the address is in the message
                    }
                }
            });
        }
    }

    // ------------------------------------------------------------------------------------------------ JSON

    /** Just enough JSON for GitHub's release answer. */
    static final class Json {
        private final String s;
        private int i;

        private Json(String s) {
            this.s = s;
        }

        static Object parse(String text) throws IOException {
            Json j = new Json(text);
            try {
                j.ws();
                Object v = j.value();
                j.ws();
                if (j.i != j.s.length()) {
                    throw new IOException("Unexpected text after the JSON answer");
                }
                return v;
            } catch (RuntimeException e) {
                throw new IOException("GitHub sent an answer that is not JSON");
            }
        }

        private void ws() {
            while (i < s.length() && Character.isWhitespace(s.charAt(i))) {
                i++;
            }
        }

        private Object value() {
            char c = s.charAt(i);
            if (c == '{') {
                i++;
                Map<String, Object> m = new LinkedHashMap<>();
                ws();
                if (s.charAt(i) == '}') {
                    i++;
                    return m;
                }
                while (true) {
                    ws();
                    String k = string();
                    ws();
                    expect(':');
                    ws();
                    m.put(k, value());
                    ws();
                    if (s.charAt(i) == ',') {
                        i++;
                        continue;
                    }
                    expect('}');
                    return m;
                }
            }
            if (c == '[') {
                i++;
                List<Object> l = new ArrayList<>();
                ws();
                if (s.charAt(i) == ']') {
                    i++;
                    return l;
                }
                while (true) {
                    ws();
                    l.add(value());
                    ws();
                    if (s.charAt(i) == ',') {
                        i++;
                        continue;
                    }
                    expect(']');
                    return l;
                }
            }
            if (c == '"') {
                return string();
            }
            if (s.startsWith("true", i)) {
                i += 4;
                return Boolean.TRUE;
            }
            if (s.startsWith("false", i)) {
                i += 5;
                return Boolean.FALSE;
            }
            if (s.startsWith("null", i)) {
                i += 4;
                return null;
            }
            int start = i;
            while (i < s.length() && "+-0123456789.eE".indexOf(s.charAt(i)) >= 0) {
                i++;
            }
            if (start == i) {
                throw new IllegalStateException("bad json");
            }
            return Double.valueOf(s.substring(start, i));
        }

        private void expect(char c) {
            if (s.charAt(i) != c) {
                throw new IllegalStateException("bad json");
            }
            i++;
        }

        private String string() {
            expect('"');
            StringBuilder sb = new StringBuilder();
            while (true) {
                char c = s.charAt(i++);
                if (c == '"') {
                    return sb.toString();
                }
                if (c != '\\') {
                    sb.append(c);
                    continue;
                }
                char e = s.charAt(i++);
                switch (e) {
                    case 'n':
                        sb.append('\n');
                        break;
                    case 't':
                        sb.append('\t');
                        break;
                    case 'r':
                        sb.append('\r');
                        break;
                    case 'b':
                        sb.append('\b');
                        break;
                    case 'f':
                        sb.append('\f');
                        break;
                    case 'u':
                        sb.append((char) Integer.parseInt(s.substring(i, i + 4), 16));
                        i += 4;
                        break;
                    default:
                        sb.append(e);
                }
            }
        }
    }
}

/** The class the Java edition's updater starts out of the downloaded jar. */
class Update {
    public static void main(String[] args) {
        Bridge.update(args);
    }
}

/** Old shortcuts that start the jar by class name keep working. */
class App {
    public static void main(String[] args) {
        Bridge.main(args);
    }
}
