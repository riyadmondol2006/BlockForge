package com.blockforge.lite.core;

import java.util.ArrayList;
import java.util.HashSet;
import java.util.LinkedHashMap;
import java.util.List;
import java.util.Map;
import java.util.Set;
import org.objectweb.asm.ClassReader;
import org.objectweb.asm.ClassVisitor;
import org.objectweb.asm.ClassWriter;
import org.objectweb.asm.MethodVisitor;
import org.objectweb.asm.Opcodes;
import org.objectweb.asm.tree.AbstractInsnNode;
import org.objectweb.asm.tree.FieldNode;
import org.objectweb.asm.tree.MethodNode;
import org.objectweb.asm.tree.analysis.Analyzer;
import org.objectweb.asm.tree.analysis.AnalyzerException;
import org.objectweb.asm.tree.analysis.BasicValue;
import org.objectweb.asm.tree.analysis.BasicVerifier;

/**
 * The class a patch works on. Only the methods a patch asks for are read with ASM; when the class is written back,
 * every other method is copied byte for byte. That keeps the cost low on Minecraft's biggest classes, at a time
 * (the game's start) when ASM's code is not compiled yet.
 */
final class Target {
    final byte[] bytes;
    final ClassReader reader;
    final String name;
    final int access;
    /** The class file's major version (50 = Java 6, 52 = Java 8, ...). */
    final int version;
    private final Map<String, MethodNode> parsed = new LinkedHashMap<String, MethodNode>();
    private final Set<String> changed = new HashSet<String>();
    private final List<MethodNode> added = new ArrayList<MethodNode>();
    private final List<FieldNode> addedFields = new ArrayList<FieldNode>();
    /** Fields ("name desc") that lose their final flag, so an added method may set them later. */
    private final Set<String> unfinal = new HashSet<String>();

    Target(byte[] bytes) {
        this.bytes = bytes;
        this.reader = new ClassReader(bytes);
        this.name = reader.getClassName();
        this.access = reader.getAccess();
        this.version = reader.readUnsignedShort(6);
    }

    /** The methods that call owner.name desc with this opcode (read with ASM, the call checked). */
    List<Object[]> calls(int opcode, String owner, String callName, String desc) {
        List<String> keys = ClassFile.methodsCalling(bytes, owner, callName, desc);
        List<Object[]> found = new ArrayList<Object[]>();
        if (keys.isEmpty()) {
            return found;
        }
        read(keys);
        for (String k : keys) {
            MethodNode m = parsed.get(k);
            if (m == null || m.instructions == null) {
                continue;
            }
            for (AbstractInsnNode n = m.instructions.getFirst(); n != null; n = n.getNext()) {
                if (Code.isCall(n, opcode, owner, callName, desc)) {
                    found.add(new Object[] {m, n});
                }
            }
        }
        return found;
    }

    /** The methods that may load this string constant (read with ASM; the caller checks the code). */
    List<MethodNode> stringUsers(String text) {
        return parsedList(ClassFile.methodsUsingString(bytes, text));
    }

    /** The methods that may read or write the field owner.name desc (read with ASM; the caller checks the code). */
    List<MethodNode> fieldUsers(String owner, String fieldName, String desc) {
        return parsedList(ClassFile.methodsUsingField(bytes, owner, fieldName, desc));
    }

    private List<MethodNode> parsedList(List<String> keys) {
        read(keys);
        List<MethodNode> l = new ArrayList<MethodNode>();
        for (String k : keys) {
            if (parsed.get(k) != null) {
                l.add(parsed.get(k));
            }
        }
        return l;
    }

    /** Every method of the class, read with ASM (only for small classes). */
    List<MethodNode> allMethods() {
        return methodsNamed(null);
    }

    /** All methods of this name (for example every constructor, "&lt;init&gt;"), read with ASM; null: all. */
    List<MethodNode> methodsNamed(final String methodName) {
        final List<String> keys = new ArrayList<String>();
        reader.accept(new ClassVisitor(Opcodes.ASM9) {
            @Override
            public MethodVisitor visitMethod(int a, String n, String d, String s, String[] e) {
                if (methodName == null || n.equals(methodName)) {
                    keys.add(n + d);
                }
                return null;
            }
        }, ClassReader.SKIP_CODE);
        read(keys);
        List<MethodNode> l = new ArrayList<MethodNode>();
        for (String k : keys) {
            l.add(parsed.get(k));
        }
        return l;
    }

    /** The access flags of a field of this class, or -1 when there is none of that name and type. */
    int fieldAccess(final String fieldName, final String desc) {
        final int[] access = {-1};
        reader.accept(new ClassVisitor(Opcodes.ASM9) {
            @Override
            public org.objectweb.asm.FieldVisitor visitField(int a, String n, String d, String s, Object v) {
                if (n.equals(fieldName) && d.equals(desc)) {
                    access[0] = a;
                }
                return null;
            }
        }, ClassReader.SKIP_CODE);
        return access[0];
    }

    /** How many fields of this type (descriptor) the class has. */
    int fieldsOfType(final String desc) {
        final int[] n = {0};
        reader.accept(new ClassVisitor(Opcodes.ASM9) {
            @Override
            public org.objectweb.asm.FieldVisitor visitField(int a, String fn, String d, String s, Object v) {
                n[0] += d.equals(desc) ? 1 : 0;
                return null;
            }
        }, ClassReader.SKIP_CODE);
        return n[0];
    }

    /** One method, read with ASM, or null when the class has none of that name and descriptor. */
    MethodNode method(String methodName, String desc) {
        String key = methodName + desc;
        if (!parsed.containsKey(key)) {
            List<String> keys = new ArrayList<String>();
            keys.add(key);
            read(keys);
        }
        return parsed.get(key);
    }

    /** Whether the class has a method of this name (any descriptor). */
    boolean hasMethodNamed(final String methodName) {
        final boolean[] found = {false};
        reader.accept(new ClassVisitor(Opcodes.ASM9) {
            @Override
            public MethodVisitor visitMethod(int a, String n, String d, String s, String[] e) {
                found[0] |= n.equals(methodName);
                return null;
            }
        }, ClassReader.SKIP_CODE);
        return found[0];
    }

    /** Marks a method that was read with {@link #method} or {@link #calls} as changed. */
    void changed(MethodNode m) {
        changed.add(m.name + m.desc);
    }

    /** Adds a method; refuses when one of that name is there already (the class was patched before). */
    void add(MethodNode m) throws Patch.Skip {
        if (hasMethodNamed(m.name)) {
            throw Patch.Skip.because("already patched");
        }
        added.add(m);
    }

    /** Adds a private static field; refuses when one of that name is there already. */
    void addField(final FieldNode f) throws Patch.Skip {
        final boolean[] found = {false};
        reader.accept(new ClassVisitor(Opcodes.ASM9) {
            @Override
            public org.objectweb.asm.FieldVisitor visitField(int a, String n, String d, String s, Object v) {
                found[0] |= n.equals(f.name);
                return null;
            }
        }, ClassReader.SKIP_CODE);
        if (found[0]) {
            throw Patch.Skip.because("already patched");
        }
        addedFields.add(f);
    }

    /**
     * Drops the final flag of a field of this class (an instance field that a patch fills later, on first use). A
     * final field may only be set in a constructor; the JVM refuses the class otherwise.
     */
    void unfinal(String fieldName, String desc) throws Patch.Skip {
        if (fieldAccess(fieldName, desc) < 0) {
            throw Patch.Skip.because("no field " + fieldName);
        }
        unfinal.add(fieldName + " " + desc);
    }

    /** The changed and the added methods (for the check). */
    List<MethodNode> touched() {
        List<MethodNode> l = new ArrayList<MethodNode>();
        for (String k : changed) {
            l.add(parsed.get(k));
        }
        l.addAll(added);
        return l;
    }

    boolean isInterface() {
        return (access & Opcodes.ACC_INTERFACE) != 0;
    }

    /** Whether the class file is new enough to need stack map frames (Java 6 class files and newer). */
    boolean needsFrames() {
        return version >= 50;
    }

    private void read(final List<String> keys) {
        reader.accept(new ClassVisitor(Opcodes.ASM9) {
            @Override
            public MethodVisitor visitMethod(int a, String n, String d, String s, String[] e) {
                String key = n + d;
                if (!keys.contains(key) || parsed.containsKey(key)) {
                    return null;
                }
                MethodNode m = new MethodNode(a, n, d, s, e);
                parsed.put(key, m);
                return m;
            }
        }, 0);
        for (String k : keys) {
            if (!parsed.containsKey(k)) {
                parsed.put(k, null);
            }
        }
    }

    /**
     * The class with the changes. The constant pool is kept and extended, unchanged methods are copied as they are,
     * frames are never computed (that would need to load game classes), so nothing of the game is loaded.
     */
    byte[] write() {
        final ClassWriter w = new ClassWriter(reader, 0) {
            @Override
            protected String getCommonSuperClass(String a, String b) {
                throw new IllegalStateException("frames would have to be computed");
            }
        };
        reader.accept(new ClassVisitor(Opcodes.ASM9, w) {
            @Override
            public org.objectweb.asm.FieldVisitor visitField(int a, String n, String d, String s, Object v) {
                return super.visitField(unfinal.contains(n + " " + d) ? a & ~Opcodes.ACC_FINAL : a, n, d, s, v);
            }

            @Override
            public MethodVisitor visitMethod(int a, String n, String d, String s, String[] e) {
                String key = n + d;
                if (changed.contains(key)) {
                    parsed.get(key).accept(w);
                    return null;
                }
                return super.visitMethod(a, n, d, s, e);
            }

            @Override
            public void visitEnd() {
                for (FieldNode f : addedFields) {
                    f.accept(w);
                }
                for (MethodNode m : added) {
                    m.accept(w);
                }
                super.visitEnd();
            }
        }, 0);
        return w.toByteArray();
    }

    /** Reads the written class again and runs ASM's verifier over the given methods; throws when something is wrong. */
    static void check(byte[] out, List<MethodNode> methods) throws AnalyzerException {
        Target again = new Target(out);
        for (MethodNode want : methods) {
            MethodNode m = again.method(want.name, want.desc);
            if (m == null) {
                throw new AnalyzerException(null, "method " + want.name + " is missing after writing");
            }
            if (m.instructions.size() > 0) {
                new Analyzer<BasicValue>(new BasicVerifier()).analyze(again.name, m);
            }
        }
    }
}
