import java.lang.reflect.Field;
import java.lang.reflect.Method;
import java.nio.file.Files;
import java.nio.file.Path;

/**
 * Starts BlockForge in a throw-away folder and measures how long the window takes to appear, failing when it takes
 * longer than the budget. Needs a display.
 *
 * usage: java -cp build/libs/BlockForge-x.y.z.jar scripts/StartupBudget.java [budget-milliseconds]
 */
public class StartupBudget {
    public static void main(String[] args) throws Exception {
        long budget = args.length > 0 ? Long.parseLong(args[0]) : 4000;
        Path work = Files.createTempDirectory("blockforge-budget");

        Class<?> app = Class.forName("com.blockforge.launcher.App");
        Method main = app.getMethod("main", String[].class);
        Field frameField = app.getField("launcherFrame");

        long start = System.currentTimeMillis();
        Thread t = new Thread(() -> {
            try {
                main.invoke(null, (Object) new String[] { "--working-dir=" + work, "--skip-setup-dialog",
                    "--no-launcher-update" });
            } catch (Exception e) {
                e.printStackTrace();
            }
        });
        t.setDaemon(true);
        t.start();

        long elapsed = -1;
        while (System.currentTimeMillis() - start < budget * 4) {
            Object frame = frameField.get(null);
            if (frame instanceof java.awt.Window && ((java.awt.Window) frame).isVisible()) {
                elapsed = System.currentTimeMillis() - start;
                break;
            }
            Thread.sleep(10);
        }

        System.out.println("Window appeared after " + elapsed + " ms (budget " + budget + " ms)");
        int code = elapsed < 0 || elapsed > budget ? 1 : 0;
        System.out.println(code == 0 ? "Within budget." : "OVER BUDGET.");
        Runtime.getRuntime().halt(code);
    }
}
