package com.blockforge.lite;

/**
 * java -cp blockforge-lite.jar com.blockforge.lite.Check [--options OPTS] client.jar [more.jar ...]
 *
 * Runs every patch against the classes of a game jar without starting the game and prints, per patch, whether it
 * would be applied or skipped and why (the same lines as the report file). For CI: run it on each new Minecraft
 * release and snapshot to see early which patches stop applying. Exit code 1 when a patch failed (its changed code
 * did not pass the check), 2 on wrong arguments.
 */
public final class Check {
    private Check() {}

    public static void main(String[] args) throws Exception {
        String options = "";
        int first = 0;
        if (args.length >= 2 && args[0].equals("--options")) {
            options = args[1];
            first = 2;
        }
        if (args.length <= first) {
            System.out.println("usage: java -cp blockforge-lite.jar com.blockforge.lite.Check [--options OPTS] "
                    + "client.jar [more.jar ...]");
            System.exit(2);
        }
        String[] jars = new String[args.length - first];
        System.arraycopy(args, first, jars, 0, jars.length);
        Class<?> m = Class.forName("com.blockforge.lite.core.Matcher", true, CoreLoader.get());
        int code = (Integer) m.getMethod("run", String.class, String[].class).invoke(null, options, jars);
        System.exit(code);
    }
}
