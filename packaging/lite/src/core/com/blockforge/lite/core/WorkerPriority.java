package com.blockforge.lite.core;

/**
 * worker-priority: on a PC with 4 or fewer logical processors, the game's shared worker threads ("Worker-Main-N":
 * world generation, lighting, chunk meshing, resource loading) run one step below normal priority, so the render
 * thread and the singleplayer server thread get the processor first when everything wants it at once. The work and
 * its results are the same; only who waits when the processor is full changes. (The analysis measured the server
 * tick median 31 % lower with the same effect made with renice on Linux.)
 *
 * Java thread priorities take effect on Windows (normal - 1 = BELOW_NORMAL; MIN_PRIORITY is avoided because it can
 * starve a thread on 2 cores); HotSpot ignores them on Linux and macOS, where the launcher can renice from outside.
 * Not a class patch: a daemon thread looks for new worker threads every few seconds (it lists the thread groups,
 * which is cheap; it takes no stack traces).
 */
final class WorkerPriority extends Tweak {
    static final String PREFIX = "Worker-Main-";
    static final int LOWER = Thread.NORM_PRIORITY - 1;
    static final long EVERY_MS = 5000L;

    WorkerPriority() {
        super("worker-priority", 1, "world and chunk worker threads one step below normal priority on 4 or fewer "
                + "processors (Windows; Linux and macOS ignore it)", new String[0]);
    }

    @Override
    String start() {
        int cpus = Runtime.getRuntime().availableProcessors();
        if (cpus > 4) {
            return cpus + " processors: not needed";
        }
        Thread t = new Thread(new Runnable() {
            public void run() {
                try {
                    while (true) {
                        Thread.sleep(EVERY_MS);
                        lowerWorkers();
                    }
                } catch (InterruptedException e) {
                    // the game is closing
                } catch (Throwable e) {
                    // a security manager or anything else: stop quietly, the threads keep their priority
                }
            }
        }, "BlockForge-Lite-priority");
        t.setDaemon(true);
        t.setPriority(Thread.NORM_PRIORITY);
        t.start();
        return null;
    }

    /** Lowers every worker thread that is above the target priority (never raises one); returns how many. */
    static int lowerWorkers() {
        ThreadGroup g = Thread.currentThread().getThreadGroup();
        while (g.getParent() != null) {
            g = g.getParent();
        }
        Thread[] all = new Thread[g.activeCount() + 16];
        int n = g.enumerate(all, true);
        int changed = 0;
        for (int i = 0; i < n; i++) {
            Thread th = all[i];
            if (th != null && th.getName().startsWith(PREFIX) && th.getPriority() > LOWER) {
                th.setPriority(LOWER);
                changed++;
            }
        }
        return changed;
    }
}
