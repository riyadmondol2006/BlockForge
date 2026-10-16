package com.blockforge.lite;

import java.io.ByteArrayOutputStream;
import java.io.File;
import java.io.IOException;
import java.io.InputStream;
import java.lang.instrument.ClassFileTransformer;
import java.net.URL;
import java.util.HashMap;
import java.util.Map;
import java.util.concurrent.atomic.AtomicLongArray;
import java.util.zip.ZipEntry;
import java.util.zip.ZipFile;
import java.util.zip.ZipInputStream;

/**
 * The private class loader of the patch code and of its copy of ASM. Its classes come from a zip file inside the agent
 * jar (so nothing on the game's class path can load them by accident), and its parent is the JDK's own loader, so it
 * can see the JDK and nothing of the game, the mod loader or the mods.
 */
final class CoreLoader extends ClassLoader {
    static final String CORE = "META-INF/blockforge-lite/core.zip";
    private static CoreLoader instance;

    static {
        registerAsParallelCapable();
    }

    private final Map<String, byte[]> classes;

    private CoreLoader(ClassLoader parent, Map<String, byte[]> classes) {
        super(parent);
        this.classes = classes;
    }

    static synchronized CoreLoader get() throws IOException {
        if (instance == null) {
            instance = new CoreLoader(jdkLoader(), read(open()));
        }
        return instance;
    }

    /** Makes the patch engine with the agent's options. */
    static ClassFileTransformer newEngine(String args, AtomicLongArray stats) throws Exception {
        Class<?> c = Class.forName("com.blockforge.lite.core.Engine", true, get());
        return (ClassFileTransformer) c.getConstructor(String.class, AtomicLongArray.class).newInstance(args, stats);
    }

    /** The platform class loader on Java 9 and newer; on Java 8 the boot loader (null), which has the whole JDK. */
    private static ClassLoader jdkLoader() {
        try {
            return (ClassLoader) ClassLoader.class.getMethod("getPlatformClassLoader").invoke(null);
        } catch (Throwable e) {
            return null;
        }
    }

    /** The nested zip, read from this agent's own jar file (found by its code source), else as a resource. */
    private static InputStream open() throws IOException {
        try {
            URL url = CoreLoader.class.getProtectionDomain().getCodeSource().getLocation();
            File jar = new File(url.toURI());
            if (jar.isFile()) {
                final ZipFile zip = new ZipFile(jar);
                ZipEntry e = zip.getEntry(CORE);
                if (e != null) {
                    return new java.io.FilterInputStream(zip.getInputStream(e)) {
                        @Override
                        public void close() throws IOException {
                            super.close();
                            zip.close();
                        }
                    };
                }
                zip.close();
            }
        } catch (Exception ignored) {
            // fall back to the resource below
        }
        InputStream in = CoreLoader.class.getResourceAsStream("/" + CORE);
        if (in == null) {
            throw new IOException(CORE + " is missing from the agent jar");
        }
        return in;
    }

    private static Map<String, byte[]> read(InputStream raw) throws IOException {
        Map<String, byte[]> m = new HashMap<String, byte[]>();
        ZipInputStream in = new ZipInputStream(raw);
        try {
            byte[] buf = new byte[16384];
            for (ZipEntry e; (e = in.getNextEntry()) != null; ) {
                String n = e.getName();
                if (e.isDirectory() || !n.endsWith(".class") || n.endsWith("module-info.class")) {
                    continue;
                }
                ByteArrayOutputStream out = new ByteArrayOutputStream(8192);
                for (int r; (r = in.read(buf)) > 0; ) {
                    out.write(buf, 0, r);
                }
                m.put(n.substring(0, n.length() - 6).replace('/', '.'), out.toByteArray());
            }
        } finally {
            in.close();
        }
        return m;
    }

    @Override
    protected Class<?> findClass(String name) throws ClassNotFoundException {
        byte[] b;
        synchronized (classes) {
            b = classes.get(name);
        }
        if (b == null) {
            throw new ClassNotFoundException(name);
        }
        return defineClass(name, b, 0, b.length);
    }
}
