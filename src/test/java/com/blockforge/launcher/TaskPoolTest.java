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
package com.blockforge.launcher;

import static org.junit.jupiter.api.Assertions.assertEquals;
import static org.junit.jupiter.api.Assertions.assertTrue;

import java.util.Set;
import java.util.concurrent.ConcurrentHashMap;
import java.util.concurrent.CountDownLatch;
import java.util.concurrent.ThreadPoolExecutor;
import java.util.concurrent.TimeUnit;

import org.junit.jupiter.api.Test;

class TaskPoolTest {
    @Test
    void runsEverythingWithAtMostItsThreadsAndLetsThemGoWhenIdle() throws Exception {
        ThreadPoolExecutor pool = App.shrinkingPool(2);
        try {
            Set<String> names = ConcurrentHashMap.newKeySet();
            CountDownLatch done = new CountDownLatch(20);
            for (int i = 0; i < 20; i++) {
                pool.execute(() -> {
                    names.add(Thread.currentThread().getName());
                    done.countDown();
                });
            }
            assertTrue(done.await(10, TimeUnit.SECONDS));
            assertTrue(names.size() <= 2, names.toString());
            assertTrue(pool.allowsCoreThreadTimeOut(), "idle threads end");
            assertEquals(30, pool.getKeepAliveTime(TimeUnit.SECONDS));
        } finally {
            pool.shutdownNow();
        }
    }

    @Test
    void rxComputationThreadsAreCappedUnlessChosen() {
        // loading App sets it, unless it was given on the command line
        assertTrue(App.TASKPOOL != null);
        String value = System.getProperty("rx3.computation-threads");
        assertTrue(value != null && Integer.parseInt(value) >= 1, String.valueOf(value));
    }
}
