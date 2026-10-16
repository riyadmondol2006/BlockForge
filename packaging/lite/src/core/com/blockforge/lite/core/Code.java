package com.blockforge.lite.core;

import java.util.List;
import org.objectweb.asm.Opcodes;
import org.objectweb.asm.tree.AbstractInsnNode;
import org.objectweb.asm.tree.AnnotationNode;
import org.objectweb.asm.tree.FieldInsnNode;
import org.objectweb.asm.tree.IntInsnNode;
import org.objectweb.asm.tree.LdcInsnNode;
import org.objectweb.asm.tree.MethodInsnNode;
import org.objectweb.asm.tree.MethodNode;
import org.objectweb.asm.tree.VarInsnNode;

/** Small helpers for reading bytecode with ASM's tree API. */
final class Code {
    /** Every method a patch adds to a game class starts with this, so it is easy to recognise in a crash report. */
    static final String PREFIX = "blockforge$lite$";

    private Code() {}

    /** The next real instruction (labels, line numbers and frames are skipped), or null. */
    static AbstractInsnNode next(AbstractInsnNode n) {
        for (n = n.getNext(); n != null && n.getOpcode() < 0; n = n.getNext()) {
            // skip
        }
        return n;
    }

    /** The previous real instruction, or null. */
    static AbstractInsnNode prev(AbstractInsnNode n) {
        for (n = n.getPrevious(); n != null && n.getOpcode() < 0; n = n.getPrevious()) {
            // skip
        }
        return n;
    }

    /**
     * The real instructions just before n, nearest first, when their opcodes are exactly these (nearest first);
     * otherwise null. Example: before(call, ILOAD, IF_ICMPGE) for "... iload; if_icmpge; call".
     */
    static AbstractInsnNode[] before(AbstractInsnNode n, int... opcodes) {
        AbstractInsnNode[] out = new AbstractInsnNode[opcodes.length];
        for (int i = 0; i < opcodes.length; i++) {
            n = n == null ? null : prev(n);
            if (n == null || n.getOpcode() != opcodes[i]) {
                return null;
            }
            out[i] = n;
        }
        return out;
    }

    /** The real instructions just after n when their opcodes are exactly these, in order; otherwise null. */
    static AbstractInsnNode[] after(AbstractInsnNode n, int... opcodes) {
        AbstractInsnNode[] out = new AbstractInsnNode[opcodes.length];
        for (int i = 0; i < opcodes.length; i++) {
            n = n == null ? null : next(n);
            if (n == null || n.getOpcode() != opcodes[i]) {
                return null;
            }
            out[i] = n;
        }
        return out;
    }

    /** Every instruction of the method that is a call to owner.name desc with this opcode (null parts: any). */
    static List<AbstractInsnNode> calls(MethodNode m, int opcode, String owner, String name, String desc) {
        List<AbstractInsnNode> out = new java.util.ArrayList<AbstractInsnNode>();
        if (m.instructions != null) {
            for (AbstractInsnNode n = m.instructions.getFirst(); n != null; n = n.getNext()) {
                if (isCall(n, opcode, owner, name, desc)) {
                    out.add(n);
                }
            }
        }
        return out;
    }

    /** Every ldc of this constant (a String, Integer, Float, Long, Double or Type) in the method. */
    static List<AbstractInsnNode> ldcs(MethodNode m, Object value) {
        List<AbstractInsnNode> out = new java.util.ArrayList<AbstractInsnNode>();
        if (m.instructions != null) {
            for (AbstractInsnNode n = m.instructions.getFirst(); n != null; n = n.getNext()) {
                if (n instanceof LdcInsnNode && value.equals(((LdcInsnNode) n).cst)) {
                    out.add(n);
                }
            }
        }
        return out;
    }

    /** The first real instruction of the method, or null. */
    static AbstractInsnNode first(MethodNode m) {
        AbstractInsnNode n = m.instructions == null ? null : m.instructions.getFirst();
        return n == null || n.getOpcode() >= 0 ? n : next(n);
    }

    /**
     * Whether the method calls into a mod loader or its API (Forge's and NeoForge's events and hooks, Fabric API,
     * Quilt). A patch whose correctness depends on nothing else touching the same state can refuse such methods.
     */
    static boolean callsModLoader(MethodNode m) {
        if (m.instructions == null) {
            return false;
        }
        for (AbstractInsnNode n = m.instructions.getFirst(); n != null; n = n.getNext()) {
            String owner = n instanceof MethodInsnNode ? ((MethodInsnNode) n).owner
                    : n instanceof FieldInsnNode ? ((FieldInsnNode) n).owner : null;
            if (owner != null && (owner.startsWith("net/minecraftforge/") || owner.startsWith("net/neoforged/")
                    || owner.startsWith("net/fabricmc/") || owner.startsWith("org/quiltmc/"))) {
                return true;
            }
        }
        return false;
    }

    static boolean isCall(AbstractInsnNode n, int opcode, String owner, String name, String desc) {
        if (!(n instanceof MethodInsnNode) || n.getOpcode() != opcode) {
            return false;
        }
        MethodInsnNode m = (MethodInsnNode) n;
        return (owner == null || m.owner.equals(owner)) && (name == null || m.name.equals(name))
                && (desc == null || m.desc.equals(desc));
    }

    static boolean isVar(AbstractInsnNode n, int opcode, int var) {
        return n instanceof VarInsnNode && n.getOpcode() == opcode && ((VarInsnNode) n).var == var;
    }

    static boolean isInt(AbstractInsnNode n, int value) {
        if (n == null) {
            return false;
        }
        int op = n.getOpcode();
        if (op >= Opcodes.ICONST_M1 && op <= Opcodes.ICONST_5) {
            return op - Opcodes.ICONST_0 == value;
        }
        if (op == Opcodes.BIPUSH || op == Opcodes.SIPUSH) {
            return ((IntInsnNode) n).operand == value;
        }
        if (op == Opcodes.LDC) {
            Object c = ((LdcInsnNode) n).cst;
            return c instanceof Integer && (Integer) c == value;
        }
        return false;
    }

    /**
     * Whether a mod changed this method with Mixin: merged or injected code, calls into Mixin's callback classes, or
     * calls to the handler methods Mixin and MixinExtras add (their names contain a '$').
     */
    static boolean changedByMixin(MethodNode m) {
        if (hasMixinAnnotation(m.visibleAnnotations) || hasMixinAnnotation(m.invisibleAnnotations)) {
            return true;
        }
        if (m.instructions == null) {
            return false;
        }
        for (AbstractInsnNode n = m.instructions.getFirst(); n != null; n = n.getNext()) {
            if (n instanceof MethodInsnNode) {
                MethodInsnNode mi = (MethodInsnNode) n;
                if (isModOwner(mi.owner) || isAddedName(mi.name)) {
                    return true;
                }
            } else if (n instanceof FieldInsnNode) {
                if (isModOwner(((FieldInsnNode) n).owner)) {
                    return true;
                }
            }
        }
        return false;
    }

    /**
     * Whether a method name looks like one Mixin or MixinExtras added (handler$..., redirect$..., md1a2b3c$...).
     * The compiler's own names with a '$' (access$000, lambda$tick$0, $values) don't count.
     */
    static boolean isAddedName(String name) {
        int d = name.indexOf('$');
        if (d < 0) {
            return false;
        }
        return d > 0 && !name.startsWith("access$") && !name.startsWith("lambda$") && !name.startsWith(PREFIX);
    }

    private static boolean isModOwner(String owner) {
        return owner.startsWith("org/spongepowered/") || owner.startsWith("com/llamalad7/mixinextras/");
    }

    private static boolean hasMixinAnnotation(List<AnnotationNode> list) {
        if (list != null) {
            for (AnnotationNode a : list) {
                if (a.desc.contains("spongepowered") || a.desc.contains("mixinextras")) {
                    return true;
                }
            }
        }
        return false;
    }
}
