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
package com.blockforge.launcher.network;

import java.io.BufferedInputStream;
import java.io.IOException;
import java.io.InputStream;
import java.io.InputStreamReader;
import java.net.SocketTimeoutException;
import java.nio.channels.Channels;
import java.nio.channels.FileChannel;
import java.nio.channels.ReadableByteChannel;
import java.nio.charset.StandardCharsets;
import java.nio.file.AtomicMoveNotSupportedException;
import java.nio.file.Files;
import java.nio.file.Path;
import java.nio.file.StandardCopyOption;
import java.util.HashMap;
import java.util.Locale;
import java.util.Map;

import com.blockforge.launcher.App;
import com.blockforge.launcher.FileSystem;
import com.blockforge.launcher.Gsons;
import com.blockforge.launcher.Network;
import com.blockforge.launcher.constants.Constants;
import com.blockforge.launcher.data.modrinth.ModrinthDownloadMetadata;
import com.blockforge.launcher.managers.LogManager;
import com.blockforge.launcher.utils.ArchiveUtils;
import com.blockforge.launcher.utils.FileFingerprints;
import com.blockforge.launcher.utils.FileUtils;
import com.blockforge.launcher.utils.Hashing;
import com.blockforge.launcher.utils.Utils;
import com.blockforge.launcher.workers.InstanceInstaller;
import com.google.common.hash.HashCode;

import okhttp3.CacheControl;
import okhttp3.Headers;
import okhttp3.HttpUrl;
import okhttp3.OkHttpClient;
import okhttp3.Request;
import okhttp3.RequestBody;
import okhttp3.Response;

import org.tukaani.xz.LZMAInputStream;

// TODO: fuck this file, it's shit, I hate it
public final class Download {
    public static final int MAX_ATTEMPTS = 3;

    // pre request
    String url;
    private String friendlyFileName;
    public Path to;
    public Path unzipTo;
    public Path extractedTo;
    public Path copyTo;
    private boolean forceDownload = false;
    private boolean ignoreFailures = false;
    private boolean deleteAfterExtract = false;
    private String hash;
    /** if the hash wasn't given but taken from what the server said (an ETag), so it's no use offline */
    private boolean hashFromServer = false;
    private Long fingerprint = null;
    public long size = -1L;
    private boolean executable = false;
    private boolean lzmaCompressed = false;
    public InstanceInstaller instanceInstaller;
    private OkHttpClient httpClient = Network.CLIENT;
    private RequestBody post = null;
    private CacheControl cacheControl = null;
    private final Map<String, String> headers = new HashMap<>();
    private ModrinthDownloadMetadata modrinthDownloadMetadata = null;

    // generated on/after request
    public Response response;

    public Download() {

    }

    public static Download build() {
        return new Download();
    }

    public boolean exists() {
        try {
            if (this.response == null) {
                this.execute();
            }

            return this.response.isSuccessful();
        } catch (DownloadException exc) {
            LogManager.logStackTrace("error fetching download from " + this.url, exc);
            if (exc.hasResponse()) {
                LogManager.error("response: " + exc.getResponse());
            }
        } catch (IOException e) {
            LogManager.logStackTrace(e);

            if (this.response != null) {
                this.response.close();
                this.response = null;
            }
        }

        return false;
    }

    public String asString() {
        try {
            this.execute();

            return this.response.body().string();
        } catch (IOException e) {
            LogManager.logStackTrace(e);

            if (this.response != null) {
                this.response.close();
                this.response = null;
            }
        }

        return null;
    }

    public <T> T asClass(Class<T> tClass) {
        try {
            return this.asClassWithThrow(tClass);
        } catch (IOException e) {
            LogManager.logStackTrace(e);

            if (this.response != null) {
                this.response.close();
                this.response = null;
            }
        }

        return null;
    }

    public <T> T asClassWithThrow(Class<T> tClass) throws IOException {
        if (this.needToDownload()) {
            this.downloadFile();
        }

        try (InputStreamReader isr = new InputStreamReader(Files.newInputStream(this.to), StandardCharsets.UTF_8)) {
            return Gsons.DEFAULT.fromJson(isr, tClass);
        }
    }

    public Download downloadTo(Path to) {
        this.to = to;

        return this;
    }

    public Download post(RequestBody post) {
        this.post = post;

        return this;
    }

    public Download unzipTo(Path unzipTo) {
        this.unzipTo = unzipTo;

        return this;
    }

    public Download forceDownload() {
        this.forceDownload = true;

        return this;
    }

    public Download ignoreFailures() {
        this.ignoreFailures = true;

        return this;
    }

    public Download deleteAfterExtract() {
        this.deleteAfterExtract = true;

        return this;
    }

    public Download hash(String hash) {
        this.hash = hash;

        return this;
    }

    public Download fingerprint(long fingerprint) {
        this.fingerprint = fingerprint;

        return this;
    }

    public Download size(long size) {
        this.size = size;

        return this;
    }

    public Download executable(boolean executable) {
        this.executable = executable;

        return this;
    }

    /**
     * The server sends the file LZMA compressed and it is unpacked while it is written. The hash is still the one of
     * the unpacked file, and the size the one of what is sent.
     */
    public Download lzmaCompressed(boolean lzmaCompressed) {
        this.lzmaCompressed = lzmaCompressed;

        return this;
    }

    boolean isLzmaCompressed() {
        return this.lzmaCompressed;
    }

    /**
     * A file of one of Mojang's Java runtimes. Most are offered LZMA compressed as well, which for Java 8 is under a
     * third of the size, so that is fetched when there is one.
     */
    public static Download forJavaRuntimeFile(
        com.blockforge.launcher.data.minecraft.JavaRuntimeManifestFileDownloads downloads, Path to) {
        com.blockforge.launcher.data.minecraft.Download raw = downloads.raw;
        com.blockforge.launcher.data.minecraft.Download lzma = downloads.lzma;
        boolean useLzma = lzma != null && lzma.url != null && !lzma.url.isEmpty();

        return new Download().setUrl(useLzma ? lzma.url : raw.url).lzmaCompressed(useLzma).downloadTo(to)
            .hash(raw.sha1).size(useLzma ? lzma.size : raw.size);
    }

    public Download copyTo(Path copyTo) {
        this.copyTo = copyTo;

        return this;
    }

    public Download setUrl(String url) {
        this.url = url;
        return this;
    }

    public Download header(String name, String value) {
        this.headers.put(name, value);
        return this;
    }

    public Download withModrinthDownloadMetadata(ModrinthDownloadMetadata modrinthDownloadMetadata) {
        this.modrinthDownloadMetadata = modrinthDownloadMetadata;
        return this;
    }

    public Download cached() {
        return cached(null);
    }

    public Download cached(CacheControl cacheControl) {
        this.httpClient = Network.CACHED_CLIENT;
        this.cacheControl = cacheControl;
        return this;
    }

    public Download withInstanceInstaller(InstanceInstaller instanceInstaller) {
        this.instanceInstaller = instanceInstaller;
        return this;
    }

    public Download withHttpClient(OkHttpClient httpClient) {
        this.httpClient = httpClient;
        return this;
    }

    public Download withFriendlyFileName(String friendlyFileName) {
        this.friendlyFileName = friendlyFileName;
        return this;
    }

    private void execute() throws IOException {
        // connection is already open, so close it first
        if (this.response != null) {
            this.response.close();
        }

        Request.Builder builder = new Request.Builder().url(this.url);

        if (this.post != null) {
            builder.post(this.post);
        }

        Headers requestHeaders = buildHeadersForUrl(this.url, this.headers, this.modrinthDownloadMetadata);
        if (requestHeaders.size() != 0) {
            builder.headers(requestHeaders);
        }

        if (this.cacheControl != null) {
            builder.cacheControl(this.cacheControl);
        }

        Request request = builder.build();
        String host = request.url().host();

        for (int attempt = 1;; attempt++) {
            try {
                RetryPolicy.awaitHost(host);
            } catch (InterruptedException e) {
                Thread.currentThread().interrupt();
                throw new IOException("Interrupted while waiting to download " + this.url, e);
            }

            try {
                this.response = httpClient.newCall(request).execute();
            } catch (IOException e) {
                if (attempt >= RetryPolicy.MAX_ATTEMPTS || !RetryPolicy.isTransientIOException(e)) {
                    throw e;
                }

                long delay = RetryPolicy.delayMillis(attempt, null);
                RetryPolicy.logRetry(this.url, "Network error (" + e.getClass().getSimpleName() + ")", attempt,
                    delay);
                pause(delay);
                continue;
            }

            if (this.response != null && (this.ignoreFailures || this.response.isSuccessful())) {
                return;
            }

            // a busy or struggling server is worth another go, anything else (404, 403 ...) won't change
            if (this.response != null && attempt < RetryPolicy.MAX_ATTEMPTS
                && RetryPolicy.isTransientStatus(this.response.code())) {
                int code = this.response.code();
                long delay = RetryPolicy.delayMillis(attempt, this.response);
                if (code == 429 || code == 503) {
                    // everything else going to that host should wait too, or they'd keep hitting the limit
                    RetryPolicy.blockHost(host, delay);
                }

                this.response.close();
                this.response = null;
                RetryPolicy.logRetry(this.url, "Server answered HTTP " + code, attempt, delay);
                pause(delay);
                continue;
            }

            if (this.response != null && this.response.code() == 429) {
                LogManager.info(this.response.headers().toString());
            }
            throw new DownloadException(this);
        }
    }

    private static void pause(long millis) throws IOException {
        try {
            Thread.sleep(millis);
        } catch (InterruptedException e) {
            Thread.currentThread().interrupt();
            throw new IOException("Interrupted while waiting to retry a download", e);
        }
    }

    static Headers buildHeadersForUrl(String url, Map<String, String> headers) {
        return buildHeadersForUrl(url, headers, null);
    }

    static Headers buildHeadersForUrl(String url, Map<String, String> headers,
        ModrinthDownloadMetadata modrinthDownloadMetadata) {
        Map<String, String> requestHeaders = new HashMap<>(headers);

        if (isCurseForgeFileDownloadUrl(url) && !hasHeader(requestHeaders, Constants.CURSEFORGE_API_KEY_HEADER)) {
            requestHeaders.put(Constants.CURSEFORGE_API_KEY_HEADER, Constants.CURSEFORGE_CORE_API_KEY);
        }

        if (modrinthDownloadMetadata != null && isModrinthFileDownloadUrl(url)
            && !hasHeader(requestHeaders, Constants.MODRINTH_DOWNLOAD_METADATA_HEADER)) {
            requestHeaders.put(Constants.MODRINTH_DOWNLOAD_METADATA_HEADER,
                modrinthDownloadMetadata.toJson());
        }

        return Headers.of(requestHeaders);
    }

    private static boolean isCurseForgeFileDownloadUrl(String url) {
        HttpUrl httpUrl = HttpUrl.parse(url);

        return httpUrl != null && Constants.CURSEFORGE_FILE_DOWNLOAD_HOST.equalsIgnoreCase(httpUrl.host());
    }

    private static boolean isModrinthFileDownloadUrl(String url) {
        HttpUrl httpUrl = HttpUrl.parse(url);

        return httpUrl != null && Constants.MODRINTH_FILE_DOWNLOAD_HOST.equalsIgnoreCase(httpUrl.host());
    }

    private static boolean hasHeader(Map<String, String> headers, String headerName) {
        return headers.keySet().stream().anyMatch(header -> header.equalsIgnoreCase(headerName));
    }

    public int code() {
        try {
            if (this.response == null) {
                this.execute();
            }
            return this.response.code();
        } catch (Exception e) {
            LogManager.logStackTrace(e);

            if (this.response != null) {
                this.response.close();
                this.response = null;
            }

            return -1;
        }
    }

    private boolean md5() {
        return this.hash != null && this.hash.length() == 32;
    }

    private boolean sha256() {
        return this.hash != null && this.hash.length() == 64;
    }

    private boolean sha512() {
        return this.hash != null && this.hash.length() == 128;
    }

    public int getResponseCode() throws IOException {
        if (this.response == null) {
            this.execute();
        }

        return this.response.code();
    }

    private String getHashFromURL() throws IOException {
        if (this.response == null) {
            this.execute();
        }

        String etag = this.response.header("ETag");
        if (etag == null) {
            return "-";
        }

        if (etag.startsWith("\"") && etag.endsWith("\"")) {
            etag = etag.substring(1, etag.length() - 1);
        }

        return etag.matches("[A-Za-z0-9]{32}") ? etag : "-";
    }

    public String getHash() {
        if (this.hash == null || this.hash.isEmpty()) {
            this.hashFromServer = true;
            try {
                this.hash = this.getHashFromURL();
            } catch (Exception e) {
                LogManager.logStackTrace(e);
                this.hash = "-";
            }
        }

        return this.hash;
    }

    public long getFilesize() {
        try {
            if (this.size == -1L) {
                if (this.response == null) {
                    this.execute();
                }
                long size = Long.parseLong(this.response.header("Content-Length", "0"));

                if (size == -1L) {
                    this.size = 0L;
                } else {
                    this.size = size;
                }
            }
        } catch (Exception e) {
            LogManager.logStackTrace(e);
            return -1;
        }

        return this.size;
    }

    public boolean needToDownload() {
        if (this.to == null || this.forceDownload) {
            return true;
        }

        if (Files.exists(this.to)) {
            // if we're ignoring failures and the file is not 0 size, then we're fine
            if (this.ignoreFailures && this.to.toFile().length() != 0) {
                return false;
            }

            // a file that hasn't changed since it last matched this hash doesn't need reading again
            if (this.fingerprint == null && this.hash != null && !this.hash.equals("-")
                && this.hash.equals(FileFingerprints.get(this.to, "hash"))) {
                return false;
            }

            // without a hash to check against, the only way to check a file is to ask the server about it, so once
            // it has been fetched or checked, trust it for as long as the file stays exactly as it was. Otherwise
            // every Play would make a request per library (Fabric and Quilt libraries have no hashes), which is
            // slow and fails when offline.
            if (this.fingerprint == null && !hasKnownHash() && this.url != null
                && this.url.equals(FileFingerprints.get(this.to, "url"))) {
                return false;
            }

            if (this.fingerprint != null) {
                try {
                    if (Hashing.murmur(this.to) == this.fingerprint) {
                        return false;
                    }
                } catch (IOException e) {
                    LogManager.error("Error getting murmur hash");
                    return false;
                }
            } else if (this.md5() && Hashing.md5(this.to).equals(Hashing.toHashCode(this.getHash()))) {
                rememberHash();
                return false;
            } else if (this.sha256() && Hashing.sha256(this.to).equals(Hashing.toHashCode(this.getHash()))) {
                rememberHash();
                return false;
            } else if (this.sha512() && Hashing.sha512(this.to).equals(Hashing.toHashCode(this.getHash()))) {
                rememberHash();
                return false;
            } else if (Hashing.sha1(this.to).equals(Hashing.toHashCode(this.getHash()))) {
                rememberHash();
                return false;
            }

            // if no hash, but filesizes match, then no need to download
            boolean need = (this.hash != null && !this.hash.equals("-"))
                || this.to.toFile().length() != this.getFilesize();
            if (!need) {
                rememberUrl();
            }
            return need;
        }

        return true;
    }

    private void downloadDirect() {
        if (size == -1L) {
            size = this.getFilesize();

            if (instanceInstaller != null && size > 0L) {
                instanceInstaller.addBytesToDownload(size);
            }
        }

        // write to a temporary file and only move it into place once it's complete, so an interrupted download
        // never leaves a half written file where a good one is expected
        Path part = this.to.resolveSibling(this.to.getFileName() + ".part");
        try {
            Files.deleteIfExists(part);

            InputStream body = this.response.body().byteStream();
            if (this.lzmaCompressed) {
                body = new LZMAInputStream(new BufferedInputStream(body, 65536));
            }

            try (FileChannel fc = FileChannel.open(part, Utils.WRITE);
                ReadableByteChannel rbc = Channels.newChannel(body)) {
                fc.transferFrom(rbc, 0, Long.MAX_VALUE);
            }

            try {
                Files.move(part, this.to, StandardCopyOption.REPLACE_EXISTING, StandardCopyOption.ATOMIC_MOVE);
            } catch (AtomicMoveNotSupportedException e) {
                Files.move(part, this.to, StandardCopyOption.REPLACE_EXISTING);
            }
        } catch (Exception e) {
            LogManager.logStackTrace("Failed to download file " + this.to, e, false);
            try {
                Files.deleteIfExists(part);
            } catch (IOException ignored) {
                // nothing more can be done about it
            }
        }
    }

    private boolean hasKnownHash() {
        return this.hash != null && !this.hash.isEmpty() && !this.hash.equals("-");
    }

    /** Notes that the file on disk is what this URL serves, for files that come without a hash. */
    private void rememberUrl() {
        if (this.url != null && this.fingerprint == null && (!hasKnownHash() || this.hashFromServer)
            && Files.exists(this.to)) {
            FileFingerprints.put(this.to, "url", this.url);
        }
    }

    /** Notes that the file on disk matches the hash it's meant to have. */
    private void rememberHash() {
        if (this.hash != null && !this.hash.equals("-")) {
            FileFingerprints.put(this.to, "hash", this.hash);
        }

        if (this.hashFromServer) {
            rememberUrl();
        }
    }

    private boolean hashMatches() {
        boolean matches = computeHashMatches();
        if (matches) {
            rememberHash();
        }
        return matches;
    }

    private boolean computeHashMatches() {
        if (Files.exists(this.to)) {
            if (this.fingerprint != null) {
                try {
                    return Hashing.murmur(this.to) == this.fingerprint;
                } catch (IOException e) {
                    LogManager.error("Error getting murmur hash");
                    return false;
                }
            } else if (this.md5()) {
                return Hashing.md5(this.to).equals(Hashing.toHashCode(this.getHash()));
            } else if (this.sha256()) {
                return Hashing.sha256(this.to).equals(Hashing.toHashCode(this.getHash()));
            } else if (this.sha512()) {
                return Hashing.sha512(this.to).equals(Hashing.toHashCode(this.getHash()));
            } else {
                return Hashing.sha1(this.to).equals(Hashing.toHashCode(this.getHash()));
            }
        }

        return false;
    }

    private boolean downloadRec(int attempt) {
        if (attempt > MAX_ATTEMPTS) {
            return false;
        }

        // if file exists, delete it
        if (Files.exists(this.to)) {
            FileUtils.delete(this.to);
        }

        // if already opened or not first attempt, open the connection
        if (this.response == null || attempt != 1) {
            try {
                this.execute();
            } catch (IOException e) {
                LogManager.logStackTrace(e);

                if (this.response != null) {
                    this.response.close();
                    this.response = null;
                }

                return false;
            }
        }

        // download the file to disk
        this.downloadDirect();

        boolean hashMatches = hashMatches();

        // if hash matches we're good
        if (hashMatches) {
            return true;
        }

        // if hash doesn't match but we're ignoring failures, then pass it if not 0 in
        // size and log a warning
        if (this.ignoreFailures && this.to.toFile().length() != 0) {
            LogManager
                .warn(String.format(Locale.ENGLISH,
                    "%s (of size %d) hash didn't match, but we're ignoring failures, so continuing",
                    this.to.getFileName(), this.to.toFile().length()));
            return true;
        }

        // if the hash doesn't match, attempt again
        LogManager.debug("Failed downloading " + this.url + " on attempt " + attempt);
        return this.downloadRec(attempt + 1);
    }

    public void copy() {
        if (this.copyTo != null) {
            if (Files.exists(this.copyTo)) {
                if (hashMatches()) {
                    return;
                }

                FileUtils.delete(this.copyTo);
            }

            if (!Files.isDirectory(this.copyTo.getParent())) {
                FileUtils.createDirectory(this.copyTo.getParent());
            }

            FileUtils.copyFile(this.to, this.copyTo, true);
        }
    }

    public void downloadFile() throws IOException {
        downloadFile(0);
    }

    public void downloadFile(int tries) throws IOException {
        if (this.instanceInstaller != null && this.instanceInstaller.isCancelled()) {
            return;
        }

        if (this.url == null || this.url.isEmpty()) {
            return;
        }

        if (!this.needToDownload()) {
            if (this.copyTo != null) {
                if (this.fingerprint != null) {
                    try {
                        if (Hashing.murmur(this.copyTo) != this.fingerprint) {
                            this.copy();
                        }
                    } catch (IOException ignored) {
                        // ignored
                    }
                } else {
                    HashCode fileHash = Hashing.EMPTY_HASH_CODE;
                    if (Files.exists(this.copyTo)) {
                        if (this.md5()) {
                            fileHash = Hashing.md5(this.copyTo);
                        } else if (this.sha256()) {
                            fileHash = Hashing.sha256(this.copyTo);
                        } else if (this.sha512()) {
                            fileHash = Hashing.sha512(this.copyTo);
                        } else {
                            fileHash = Hashing.sha1(this.copyTo);
                        }
                    }

                    if (!fileHash.equals(Hashing.toHashCode(this.getHash()))) {
                        this.copy();
                    }
                }
            }

            runPostProcessors();
            return;
        }

        // open the connection if not already opened
        if (this.response == null) {
            try {
                this.execute();
            } catch (IOException e) {
                // if timeout, attempt to download again
                if (e instanceof SocketTimeoutException && tries < 3) {
                    LogManager.warn(String.format("Failed to download %s from %s due to timeout. Attempting again.",
                        this.to.getFileName().toString(), this.url));
                    downloadFile(tries++);
                    return;
                }

                if (this.instanceInstaller != null) {
                    this.instanceInstaller.cancel(true);
                }

                throw e;
            }
        }

        Path oldPath = null;
        if (Files.exists(this.to)) {
            oldPath = this.to.resolveSibling(this.to.getFileName().toString() + ".bak");
            FileUtils.moveFile(this.to, oldPath, true);
        }

        if (Files.exists(this.to) && Files.isRegularFile(this.to)) {
            FileUtils.delete(this.to);
        }

        if (!Files.isDirectory(this.to.getParent())) {
            FileUtils.createDirectory(this.to.getParent());
        }

        HashCode expected = null;

        if (this.fingerprint == null) {
            expected = Hashing.toHashCode(this.getHash());
        }

        if ((this.ignoreFailures && this.to.toFile().length() != 0)
            || (expected != null && expected.equals(Hashing.EMPTY_HASH_CODE))) {
            if (this.response.isSuccessful()) {
                this.downloadDirect();

                if (!this.ignoreFailures) {
                    rememberUrl();
                }
            }
        } else {
            boolean downloaded = this.downloadRec(1);

            if (!downloaded) {
                if (this.response != null && this.response.header("content-type").contains("text/html")) {
                    LogManager.error(
                        "The response from this request was a HTML response. This is usually caused by an antivirus or firewall software intercepting and rewriting the response. The response is below.");

                    LogManager.error(new String(Files.readAllBytes(this.to), StandardCharsets.UTF_8));
                }

                FileUtils.copyFile(this.to, FileSystem.FAILED_DOWNLOADS);
                if (fingerprint != null) {
                    LogManager.error("Error downloading " + this.to.getFileName() + " from " + this.url + ". Expected"
                        + " fingerprint of " + fingerprint.toString() + " ("
                        + (this.size == 0 ? "with unknown size" : "with size of" + this.size) + ") but got "
                        + Hashing.murmur(this.to) + " (with size of "
                        + (Files.exists(this.to) ? Files.size(this.to) : 0)
                        + ") instead. Copied to FailedDownloads folder & cancelling install! ("
                        + App.settings.connectionTimeout + "/" + App.settings.concurrentConnections + ")");
                } else {
                    LogManager.error("Error downloading " + this.to.getFileName() + " from " + this.url + ". Expected"
                        + " hash of " + expected.toString() + " ("
                        + (this.size == 0 ? "with unknown size" : "with size of" + this.size) + ") but got "
                        + (this.md5() ? Hashing.md5(this.to)
                        : (this.sha256() ? Hashing.sha256(this.to)
                            : (this.sha512() ? Hashing.sha512(this.to) : Hashing.sha1(this.to))))
                        + " (with size of " + (Files.exists(this.to) ? Files.size(this.to) : 0)
                        + ") instead. Copied to FailedDownloads folder & cancelling install! ("
                        + App.settings.connectionTimeout + "/" + App.settings.concurrentConnections + ")");
                }
                if (this.instanceInstaller != null) {
                    this.instanceInstaller.cancel(true);
                }
            }

            if (downloaded && this.copyTo != null) {
                if (this.fingerprint != null) {
                    try {
                        if (Hashing.murmur(this.copyTo) != this.fingerprint) {
                            this.copy();
                        }
                    } catch (IOException ignored) {
                        // ignored
                    }
                } else {
                    HashCode fileHash2 = Hashing.EMPTY_HASH_CODE;
                    if (Files.exists(this.copyTo)) {
                        if (this.md5()) {
                            fileHash2 = Hashing.md5(this.copyTo);
                        } else if (this.sha256()) {
                            fileHash2 = Hashing.sha256(this.copyTo);
                        } else if (this.sha512()) {
                            fileHash2 = Hashing.sha512(this.copyTo);
                        } else {
                            fileHash2 = Hashing.sha1(this.copyTo);
                        }
                    }

                    if (!fileHash2.equals(expected)) {
                        this.copy();
                    }
                }
            }
        }

        runPostProcessors();

        if (oldPath != null && Files.exists(oldPath)) {
            FileUtils.delete(oldPath);
        }
    }

    public void runPostProcessors() {
        if (this.response != null) {
            this.response.close();
        }

        if (Files.exists(this.to) && this.unzipTo != null) {
            FileUtils.createDirectory(this.unzipTo);

            ArchiveUtils.extract(this.to, this.unzipTo);

            if (this.deleteAfterExtract) {
                FileUtils.delete(this.to);
            }
        }

        if (Files.exists(this.to) && this.executable) {
            this.to.toFile().setExecutable(this.executable);
        }
    }

    public String getPrintableFileName() {
        if (this.friendlyFileName != null) {
            return this.friendlyFileName;
        }

        return this.to.getFileName().toString();
    }

    @Override
    public boolean equals(Object other) {
        if (other instanceof Download) {
            try {
                return Files.isSameFile(this.to, ((Download) other).to);
            } catch (IOException e) {
                return false;
            }
        } else {
            return false;
        }
    }

    @Override
    public int hashCode() {
        return this.to.hashCode();
    }
}
