package com.blockforge.lite.core;

import java.util.concurrent.CountDownLatch;

/** Self-test of worker-priority (a tweak, not a class patch). */
final class WorkerPriorityTest extends PatchTest {
    void lowersOnlyWorkers() throws Exception {
        final CountDownLatch done = new CountDownLatch(1);
        Runnable wait = new Runnable() {
            public void run() {
                try {
                    done.await();
                } catch (InterruptedException ignored) {
                    // finished
                }
            }
        };
        Thread worker = new Thread(wait, "Worker-Main-42");
        Thread render = new Thread(wait, "Render thread");
        Thread low = new Thread(wait, "Worker-Main-43");
        worker.setPriority(Thread.NORM_PRIORITY);
        render.setPriority(Thread.NORM_PRIORITY);
        low.setPriority(Thread.MIN_PRIORITY);
        worker.start();
        render.start();
        low.start();
        try {
            WorkerPriority.lowerWorkers();
            expect(worker.getPriority() == Thread.NORM_PRIORITY - 1, "a worker goes one step below normal");
            expect(render.getPriority() == Thread.NORM_PRIORITY, "other threads keep theirs");
            expect(low.getPriority() == Thread.MIN_PRIORITY, "a lower priority is never raised");
            expect(WorkerPriority.lowerWorkers() == 0, "nothing to do the second time");
        } finally {
            done.countDown();
        }
    }

    void reported() throws Exception {
        String report = reportPath("prio");
        // the engine starts with the first Minecraft class the agent sees
        factory.apply("quiet,report=" + report).transform(null, "net/minecraft/X", null, null, new byte[0]);
        String l = line(report, "worker-priority");
        boolean small = Runtime.getRuntime().availableProcessors() <= 4;
        expect(small ? l.contains("\tapplied\t") : l.contains("processors: not needed"),
                "reported for " + Runtime.getRuntime().availableProcessors() + " processors: " + l);
        String off = reportPath("prio-off");
        factory.apply("quiet,off=worker-priority,report=" + off).transform(null, "net/minecraft/X", null, null,
                new byte[0]);
        expect(line(off, "worker-priority").contains("off="), "off=worker-priority: off");
    }
}
