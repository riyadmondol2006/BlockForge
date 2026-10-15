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
package com.blockforge.launcher.network;

import static org.junit.jupiter.api.Assertions.assertArrayEquals;
import static org.junit.jupiter.api.Assertions.assertEquals;
import static org.junit.jupiter.api.Assertions.assertFalse;
import static org.junit.jupiter.api.Assertions.assertTrue;

import java.io.IOException;
import java.io.OutputStream;
import java.net.InetSocketAddress;
import java.nio.charset.StandardCharsets;
import java.nio.file.Files;
import java.nio.file.Path;
import java.security.MessageDigest;
import java.util.concurrent.atomic.AtomicInteger;

import org.junit.jupiter.api.AfterEach;
import org.junit.jupiter.api.BeforeEach;
import org.junit.jupiter.api.Test;
import org.junit.jupiter.api.io.TempDir;

import com.blockforge.launcher.App;
import com.blockforge.launcher.data.Settings;
import com.sun.net.httpserver.HttpServer;

import okhttp3.OkHttpClient;

/**
 * Downloads from a local server that misbehaves in the ways real servers do.
 */
class DownloadRetryTest {
    private static final byte[] BODY = "BlockForge test file contents".getBytes(StandardCharsets.UTF_8);

    private HttpServer server;
    private String baseUrl;
    private final AtomicInteger requests = new AtomicInteger();

    @TempDir
    Path temp;

    @BeforeEach
    void start() throws IOException {
        if (App.settings == null) {
            App.settings = new Settings();
        }
        server = HttpServer.create(new InetSocketAddress("127.0.0.1", 0), 0);
        baseUrl = "http://127.0.0.1:" + server.getAddress().getPort();
    }

    @AfterEach
    void stop() {
        server.stop(0);
    }

    private static String sha1(byte[] bytes) throws Exception {
        StringBuilder sb = new StringBuilder();
        for (byte b : MessageDigest.getInstance("SHA-1").digest(bytes)) {
            sb.append(String.format("%02x", b));
        }
        return sb.toString();
    }

    /** answers with the given status for the first {@code failures} requests, then serves the file */
    private void serveAfterFailures(int failures, int status, String retryAfter) {
        server.createContext("/file", exchange -> {
            if (requests.incrementAndGet() <= failures) {
                if (retryAfter != null) {
                    exchange.getResponseHeaders().add("Retry-After", retryAfter);
                }
                exchange.sendResponseHeaders(status, -1);
                exchange.close();
                return;
            }
            exchange.sendResponseHeaders(200, BODY.length);
            try (OutputStream out = exchange.getResponseBody()) {
                out.write(BODY);
            }
        });
        server.start();
    }

    private Download download(Path target) throws Exception {
        return Download.build().setUrl(baseUrl + "/file").hash(sha1(BODY)).size(BODY.length).downloadTo(target)
            .withHttpClient(new OkHttpClient());
    }

    @Test
    void recoversFromRateLimiting() throws Exception {
        serveAfterFailures(2, 429, "1");
        Path target = temp.resolve("file.bin");

        download(target).downloadFile();

        assertArrayEquals(BODY, Files.readAllBytes(target));
        assertEquals(3, requests.get());
    }

    @Test
    void recoversFromServerErrors() throws Exception {
        serveAfterFailures(1, 503, "0");
        Path target = temp.resolve("file.bin");

        download(target).downloadFile();

        assertArrayEquals(BODY, Files.readAllBytes(target));
        assertEquals(2, requests.get());
    }

    @Test
    void doesNotRetryWhenTheFileIsMissing() throws Exception {
        serveAfterFailures(100, 404, null);
        Path target = temp.resolve("file.bin");

        try {
            download(target).downloadFile();
        } catch (IOException expected) {
            // a missing file is an error
        }

        assertFalse(Files.exists(target));
        assertEquals(1, requests.get());
    }

    @Test
    void leavesNoPartialFileBehind() throws Exception {
        serveAfterFailures(0, 200, null);
        Path target = temp.resolve("file.bin");

        download(target).downloadFile();

        assertTrue(Files.exists(target));
        assertFalse(Files.exists(temp.resolve("file.bin.part")));
    }

    @Test
    void parsesRetryAfterHeaders() {
        assertEquals(3000, RetryPolicy.parseRetryAfter("3"));
        assertEquals(-1, RetryPolicy.parseRetryAfter(null));
        assertEquals(-1, RetryPolicy.parseRetryAfter("soon"));
        assertTrue(RetryPolicy.parseRetryAfter("Wed, 21 Oct 2099 07:28:00 GMT") > 0);
    }

    @Test
    void backoffGrowsAndIsCapped() {
        long first = RetryPolicy.delayMillis(1, null);
        long later = RetryPolicy.delayMillis(5, null);
        assertTrue(first >= 750 && first <= 1200);
        assertTrue(later > first);
        assertTrue(RetryPolicy.delayMillis(20, null) <= 45_000);
    }
}
