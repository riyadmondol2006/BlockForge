package com.blockforge.lite.core;

import java.io.ByteArrayOutputStream;
import java.io.InputStream;
import java.util.ArrayList;
import java.util.HashMap;
import java.util.List;
import java.util.Map;
import java.util.WeakHashMap;
import org.objectweb.asm.ClassReader;
import org.objectweb.asm.ClassVisitor;
import org.objectweb.asm.Opcodes;
import org.objectweb.asm.Type;
import org.objectweb.asm.tree.AbstractInsnNode;
import org.objectweb.asm.tree.ClassNode;
import org.objectweb.asm.tree.FieldInsnNode;
import org.objectweb.asm.tree.FieldNode;
import org.objectweb.asm.tree.FrameNode;
import org.objectweb.asm.tree.InvokeDynamicInsnNode;
import org.objectweb.asm.tree.LdcInsnNode;
import org.objectweb.asm.tree.LineNumberNode;
import org.objectweb.asm.tree.MethodInsnNode;
import org.objectweb.asm.tree.MethodNode;
import org.objectweb.asm.tree.MultiANewArrayInsnNode;
import org.objectweb.asm.tree.TypeInsnNode;

/**
 * Code a patch adds to a game class, written in Java instead of instructions: a template class in {@code tpl/},
 * compiled against stand-ins of the game's classes ({@code src/stubs}, with the game's names, 26.x for now), and never
 * loaded.
 * {@link #copy} copies its static fields and methods into the target as private static members (all named with
 * {@link Code#PREFIX}), turning references to the template into references to the target. javac wrote the stack map
 * frames, so nothing is computed here.
 *
 * Before anything is copied, every class, method and field of the game the template uses is looked up in the running
 * game (the class files its class loader has): it must be there, public, with the same descriptor, static or not and
 * class or interface as the template expects. Otherwise the patch is skipped, so a renamed member in a new Minecraft
 * version can only turn a patch off. Template rules: only static members named with the prefix, no lambdas, no string
 * concatenation with +, no other template or agent classes; wrap the work in try/catch where a failure must not reach
 * the game.
 */
final class Template {
    private Template() {}

    static final String PACKAGE = "com/blockforge/lite/core/tpl/";

    /** Copies the template into the target; returns the copied methods (by name). */
    static Map<String, MethodNode> copy(Target t, Patch.Ctx ctx, String template) throws Patch.Skip {
        ClassNode c = read(template);
        String from = c.name;
        List<String> problems = new ArrayList<String>();
        Map<String, MethodNode> out = new HashMap<String, MethodNode>();
        List<FieldNode> fields = new ArrayList<FieldNode>();
        for (FieldNode f : c.fields) {
            if ((f.access & Opcodes.ACC_STATIC) == 0 || !f.name.startsWith(Code.PREFIX) || f.value != null) {
                throw new IllegalStateException("template field " + f.name + " is not a plain static field");
            }
            fields.add(new FieldNode(Opcodes.ACC_PRIVATE | Opcodes.ACC_STATIC | Opcodes.ACC_SYNTHETIC
                    | (f.access & Opcodes.ACC_VOLATILE), f.name, f.desc, null, null));
        }
        List<MethodNode> methods = new ArrayList<MethodNode>();
        for (MethodNode m : c.methods) {
            if (m.name.equals("<init>")) {
                continue;
            }
            if ((m.access & Opcodes.ACC_STATIC) == 0 || !m.name.startsWith(Code.PREFIX)) {
                throw new IllegalStateException("template method " + m.name + " is not static or not prefixed");
            }
            MethodNode n = new MethodNode(Opcodes.ACC_PRIVATE | Opcodes.ACC_STATIC | Opcodes.ACC_SYNTHETIC, m.name,
                    m.desc, null, m.exceptions == null ? null : m.exceptions.toArray(new String[0]));
            m.accept(n);
            n.access = Opcodes.ACC_PRIVATE | Opcodes.ACC_STATIC | Opcodes.ACC_SYNTHETIC;
            n.localVariables = null;
            n.parameters = null;
            n.visibleAnnotations = null;
            n.invisibleAnnotations = null;
            for (AbstractInsnNode i = n.instructions.getFirst(); i != null; ) {
                AbstractInsnNode next = i.getNext();
                if (i instanceof LineNumberNode || (i instanceof FrameNode && !t.needsFrames())) {
                    n.instructions.remove(i);
                } else {
                    retarget(i, from, t.name, problems);
                }
                i = next;
            }
            if (!t.needsFrames()) {
                n.maxStack = Math.max(n.maxStack, 1);
            }
            methods.add(n);
            out.put(n.name, n);
        }
        if (!problems.isEmpty()) {
            throw new IllegalStateException("template " + template + ": " + problems);
        }
        // everything of the game the code uses must be in this game
        Lookup look = lookup(ctx.loader);
        for (MethodNode m : methods) {
            for (AbstractInsnNode i = m.instructions.getFirst(); i != null; i = i.getNext()) {
                String missing = look.check(i, t.name);
                if (missing != null) {
                    throw Patch.Skip.because("this game has no " + missing);
                }
            }
        }
        for (FieldNode f : fields) {
            t.addField(f);
        }
        for (MethodNode m : methods) {
            t.add(m);
        }
        return out;
    }

    private static void retarget(AbstractInsnNode i, String from, String to, List<String> problems) {
        if (i instanceof MethodInsnNode) {
            MethodInsnNode m = (MethodInsnNode) i;
            if (m.owner.equals(from)) {
                m.owner = to;
            } else if (m.owner.startsWith("com/blockforge/")) {
                problems.add("calls " + m.owner);
            }
        } else if (i instanceof FieldInsnNode) {
            FieldInsnNode f = (FieldInsnNode) i;
            if (f.owner.equals(from)) {
                f.owner = to;
            } else if (f.owner.startsWith("com/blockforge/")) {
                problems.add("uses " + f.owner);
            }
        } else if (i instanceof InvokeDynamicInsnNode) {
            problems.add("invokedynamic " + ((InvokeDynamicInsnNode) i).name);
        } else if (i instanceof TypeInsnNode && ((TypeInsnNode) i).desc.contains("com/blockforge/")) {
            problems.add("uses type " + ((TypeInsnNode) i).desc);
        } else if (i instanceof LdcInsnNode && ((LdcInsnNode) i).cst instanceof Type) {
            problems.add("class literal " + ((LdcInsnNode) i).cst);
        }
    }

    private static ClassNode read(String template) {
        String path = PACKAGE + template + ".class";
        try {
            ClassLoader l = Template.class.getClassLoader();
            InputStream in = l == null ? ClassLoader.getSystemResourceAsStream(path) : l.getResourceAsStream(path);
            if (in == null) {
                throw new IllegalStateException(path + " is missing");
            }
            ClassNode c = new ClassNode();
            new ClassReader(bytes(in)).accept(c, 0);
            return c;
        } catch (java.io.IOException e) {
            throw new IllegalStateException(path + ": " + e);
        }
    }

    static byte[] bytes(InputStream in) throws java.io.IOException {
        try {
            ByteArrayOutputStream out = new ByteArrayOutputStream(8192);
            byte[] buf = new byte[8192];
            for (int r; (r = in.read(buf)) > 0; ) {
                out.write(buf, 0, r);
            }
            return out.toByteArray();
        } finally {
            in.close();
        }
    }

    private static final Map<ClassLoader, Lookup> LOOKUPS = new WeakHashMap<ClassLoader, Lookup>();

    private static synchronized Lookup lookup(ClassLoader loader) {
        Lookup l = LOOKUPS.get(loader);
        if (l == null) {
            l = new Lookup(loader);
            LOOKUPS.put(loader, l);
        }
        return l;
    }

    /** Finds the game's classes and members through the class files its class loader has (nothing is loaded). */
    static final class Lookup {
        private final ClassLoader loader;
        /** Class name -> what the patches need to know of it; null: the game has no such class. */
        private final Map<String, Info> classes = new HashMap<String, Info>();

        Lookup(ClassLoader loader) {
            this.loader = loader;
        }

        /** What the instruction needs that this game does not have, or null. */
        synchronized String check(AbstractInsnNode i, String self) {
            if (i instanceof MethodInsnNode) {
                MethodInsnNode m = (MethodInsnNode) i;
                if (skip(m.owner, self)) {
                    return null;
                }
                Info c = cls(m.owner);
                if (c == null) {
                    return "class " + m.owner;
                }
                boolean itf = (c.access & Opcodes.ACC_INTERFACE) != 0;
                if (itf != (m.getOpcode() == Opcodes.INVOKEINTERFACE || m.getOpcode() == Opcodes.INVOKESTATIC && m.itf)
                        && m.getOpcode() != Opcodes.INVOKESPECIAL) {
                    return (itf ? "class " : "interface ") + m.owner;
                }
                if ((c.access & Opcodes.ACC_PUBLIC) == 0) {
                    return "public class " + m.owner;
                }
                // constructors are not inherited
                Integer access = method(m.owner, m.name, m.desc, m.name.equals("<init>") ? 30 : 0);
                if (access == null || (access & Opcodes.ACC_PUBLIC) == 0
                        || ((access & Opcodes.ACC_STATIC) != 0) != (m.getOpcode() == Opcodes.INVOKESTATIC)) {
                    return "method " + m.owner + "." + m.name + m.desc;
                }
            } else if (i instanceof FieldInsnNode) {
                FieldInsnNode f = (FieldInsnNode) i;
                if (skip(f.owner, self)) {
                    return null;
                }
                Integer access = field(f.owner, f.name, f.desc, 0);
                boolean isStatic = f.getOpcode() == Opcodes.GETSTATIC || f.getOpcode() == Opcodes.PUTSTATIC;
                boolean put = f.getOpcode() == Opcodes.PUTSTATIC || f.getOpcode() == Opcodes.PUTFIELD;
                boolean finalField = access != null && (access & Opcodes.ACC_FINAL) != 0;
                if (access == null || (access & Opcodes.ACC_PUBLIC) == 0
                        || ((access & Opcodes.ACC_STATIC) != 0) != isStatic || put && finalField) {
                    return "field " + f.owner + "." + f.name + " " + f.desc;
                }
            } else if (i instanceof TypeInsnNode || i instanceof MultiANewArrayInsnNode) {
                String d = i instanceof TypeInsnNode ? ((TypeInsnNode) i).desc : ((MultiANewArrayInsnNode) i).desc;
                Type ty = d.startsWith("[") ? Type.getType(d).getElementType() : Type.getObjectType(d);
                if (ty.getSort() == Type.OBJECT && !skip(ty.getInternalName(), self)) {
                    Info c = cls(ty.getInternalName());
                    if (c == null || (c.access & Opcodes.ACC_PUBLIC) == 0 && !c.publicNested) {
                        return "class " + ty.getInternalName();
                    }
                }
            }
            return null;
        }

        private static boolean skip(String owner, String self) {
            return owner.equals(self) || owner.startsWith("java/");
        }

        /** The access of the method in the class or the classes and interfaces above it, or null. */
        private Integer method(String owner, String name, String desc, int depth) {
            return member(owner, name + " " + desc, true, depth);
        }

        private Integer field(String owner, String name, String desc, int depth) {
            return member(owner, name + " " + desc, false, depth);
        }

        private Integer member(String owner, String key, boolean method, int depth) {
            Info c = depth > 30 ? null : cls(owner);
            if (c == null) {
                return null;
            }
            Integer a = (method ? c.methods : c.fields).get(key);
            if (a != null || depth >= 30) {
                return a;
            }
            List<String> up = new ArrayList<String>(c.interfaces);
            if (c.superName != null) {
                up.add(0, c.superName);
            }
            for (String s : up) {
                a = s.startsWith("java/") ? null : member(s, key, method, depth + 1);
                if (a != null) {
                    return a;
                }
            }
            return null;
        }

        /** The class's access, supertypes and members, or null when the game has no such class file. */
        private Info cls(final String name) {
            if (classes.containsKey(name)) {
                return classes.get(name);
            }
            Info c = null;
            try {
                InputStream in = loader == null ? ClassLoader.getSystemResourceAsStream(name + ".class")
                        : loader.getResourceAsStream(name + ".class");
                if (in != null) {
                    final Info info = new Info();
                    new ClassReader(bytes(in)).accept(new ClassVisitor(Opcodes.ASM9) {
                        @Override
                        public void visit(int v, int a, String n, String sig, String sup, String[] itfs) {
                            info.access = a;
                            info.superName = sup;
                            if (itfs != null) {
                                info.interfaces.addAll(java.util.Arrays.asList(itfs));
                            }
                        }

                        @Override
                        public void visitInnerClass(String n, String outer, String inner, int a) {
                            info.publicNested |= n.equals(name) && (a & Opcodes.ACC_PUBLIC) != 0;
                        }

                        @Override
                        public org.objectweb.asm.FieldVisitor visitField(int a, String n, String d, String sig,
                                Object v) {
                            info.fields.put(n + " " + d, a);
                            return null;
                        }

                        @Override
                        public org.objectweb.asm.MethodVisitor visitMethod(int a, String n, String d, String sig,
                                String[] e) {
                            info.methods.put(n + " " + d, a);
                            return null;
                        }
                    }, ClassReader.SKIP_CODE | ClassReader.SKIP_DEBUG | ClassReader.SKIP_FRAMES);
                    c = info;
                }
            } catch (Throwable e) {
                c = null;
            }
            classes.put(name, c);
            return c;
        }
    }

    /** What a lookup keeps of a class. */
    static final class Info {
        int access;
        String superName;
        final List<String> interfaces = new ArrayList<String>();
        boolean publicNested;
        final Map<String, Integer> methods = new HashMap<String, Integer>();
        final Map<String, Integer> fields = new HashMap<String, Integer>();
    }
}
