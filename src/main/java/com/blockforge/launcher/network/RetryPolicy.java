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

import java.io.EOFException;
import java.io.IOException;
import java.net.ConnectException;
import java.net.NoRouteToHostException;
import java.net.SocketException;
import java.net.SocketTimeoutException;
import java.net.UnknownHostException;
import java.time.Duration;
import java.time.Instant;
import java.time.ZonedDateTime;
import java.time.format.DateTimeFormatter;
import java.util.Map;
import java.util.concurrent.ConcurrentHashMap;
import java.util.concurrent.ThreadLocalRandom;

import com.blockforge.launcher.managers.LogManager;

import okhttp3.Response;

/**
 * Decides which download failures are worth trying again and how long to wait first. A busy server answering
 * "slow down" (429) or "try again" (503) is normal, so it shouldn't end an installation.
 */
final class RetryPolicy {
    /** how many times a request is tried in total */
    static final int MAX_ATTEMPTS = 5;

    private static final long MAX_DELAY_MS = 30_000;
    private static final long BASE_DELAY_MS = 750;

    /** per host, the time before which no new requests should be sent because the host asked us to wait */
    private static final Map<String, Long> BLOCKED_UNTIL = new ConcurrentHashMap<>();

    private RetryPolicy() {
    }

    static boolean isTransientStatus(int code) {
        return code == 408 || code == 425 || code == 429 || code == 500 || code == 502 || code == 503
            || code == 504;
    }

    /**
     * Network errors that tend to go away by themselves. Being offline (unknown host, no route, refused connection)
     * isn't one of them: those fail straight away so that offline use isn't slowed down.
     */
    static boolean isTransientIOException(IOException e) {
        if (e instanceof UnknownHostException || e instanceof NoRouteToHostException
            || e instanceof ConnectException) {
            return false;
        }

        return e instanceof SocketTimeoutException || e instanceof SocketException || e instanceof EOFException
            || e instanceof java.io.InterruptedIOException;
    }

    /** How long to wait before the next attempt (attempt is the one that just failed, starting at 1). */
    static long delayMillis(int attempt, Response response) {
        long retryAfter = response == null ? -1 : parseRetryAfter(response.header("Retry-After"));
        if (retryAfter >= 0) {
            return Math.min(retryAfter, MAX_DELAY_MS);
        }

        long exponential = Math.min(MAX_DELAY_MS, BASE_DELAY_MS << Math.min(attempt - 1, 6));
        // a random share is added so many downloads that failed together don't all come back together
        return exponential + ThreadLocalRandom.current().nextLong(exponential / 2 + 1);
    }

    static long parseRetryAfter(String header) {
        if (header == null || header.trim().isEmpty()) {
            return -1;
        }

        String value = header.trim();
        try {
            return Math.max(0, Long.parseLong(value)) * 1000;
        } catch (NumberFormatException ignored) {
            // not a number of seconds, so it may be a date
        }

        try {
            ZonedDateTime date = ZonedDateTime.parse(value, DateTimeFormatter.RFC_1123_DATE_TIME);
            return Math.max(0, Duration.between(Instant.now(), date.toInstant()).toMillis());
        } catch (RuntimeException e) {
            return -1;
        }
    }

    /** Tells every download to that host to hold off for a while. */
    static void blockHost(String host, long millis) {
        long until = System.currentTimeMillis() + millis;
        BLOCKED_UNTIL.merge(host, until, Math::max);
    }

    /** Waits out any pause that the host asked for. */
    static void awaitHost(String host) throws InterruptedException {
        Long until = BLOCKED_UNTIL.get(host);
        if (until == null) {
            return;
        }

        long wait = until - System.currentTimeMillis();
        if (wait > 0) {
            Thread.sleep(wait);
        } else {
            BLOCKED_UNTIL.remove(host, until);
        }
    }

    static void logRetry(String url, String reason, int attempt, long delayMs) {
        LogManager.info(String.format("%s while downloading %s, trying again in %.1fs (attempt %d of %d)", reason, url,
            delayMs / 1000.0, attempt + 1, MAX_ATTEMPTS));
    }
}
