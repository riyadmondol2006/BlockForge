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

import java.util.LinkedHashMap;
import java.util.Map;
import java.util.function.LongSupplier;

/**
 * Notes how long each step between pressing Play and the game starting took, so a slow launch can be explained from
 * the log with one line instead of guessed at.
 */
public final class LaunchTimings {
    private final LongSupplier clock;
    private final long start;
    private long last;
    private final Map<String, Long> steps = new LinkedHashMap<>();

    public LaunchTimings() {
        this(() -> System.nanoTime() / 1_000_000L);
    }

    LaunchTimings(LongSupplier millisClock) {
        this.clock = millisClock;
        this.start = millisClock.getAsLong();
        this.last = this.start;
    }

    /** Marks the end of a step that started when the previous one ended. */
    public synchronized void step(String name) {
        long now = clock.getAsLong();
        steps.merge(name, now - last, Long::sum);
        last = now;
    }

    public synchronized long total() {
        return last - start;
    }

    /** "1234 ms (libraries 12 ms, assets 1200 ms, ...)", leaving out steps that took no time worth mentioning */
    public synchronized String summary() {
        StringBuilder sb = new StringBuilder().append(total()).append(" ms");
        StringBuilder parts = new StringBuilder();
        for (Map.Entry<String, Long> e : steps.entrySet()) {
            if (e.getValue() < 1) {
                continue;
            }
            if (parts.length() > 0) {
                parts.append(", ");
            }
            parts.append(e.getKey()).append(' ').append(e.getValue()).append(" ms");
        }
        if (parts.length() > 0) {
            sb.append(" (").append(parts).append(')');
        }
        return sb.toString();
    }
}
