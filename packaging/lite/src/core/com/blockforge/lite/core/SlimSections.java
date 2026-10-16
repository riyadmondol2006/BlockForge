package com.blockforge.lite.core;

import java.util.ArrayList;
import java.util.List;
import org.objectweb.asm.Opcodes;
import org.objectweb.asm.tree.AbstractInsnNode;
import org.objectweb.asm.tree.FieldInsnNode;
import org.objectweb.asm.tree.InsnList;
import org.objectweb.asm.tree.InsnNode;
import org.objectweb.asm.tree.LdcInsnNode;
import org.objectweb.asm.tree.MethodInsnNode;
import org.objectweb.asm.tree.MethodNode;
import org.objectweb.asm.tree.TypeInsnNode;

/**
 * slim-sections: every 16x16x16 piece of a loaded world keeps its blocks and biomes in PalettedContainers, and each
 * container carries a ThreadingDetector (a debug guard against two threads changing it at once: a semaphore, a lock
 * and their parts, about 128 bytes). With hundreds of thousands of containers that is 10 MB (1.21.1) to 39 MB (26.3)
 * of the heap, plus garbage every time a section is copied for drawing (measured by the analysis, A2 #1).
 *
 * The patch stops creating the guard (the field stays null) and makes acquire()/release(), the only code that uses
 * it, return at once - what Lithium's default "chunk.no_locking" does. Correct code never trips the guard, so nothing
 * changes for the game; a buggy mod that writes a section from two threads would no longer get the guard's crash
 * report.
 *
 * Target, the same in 1.18 - 26.x (and with Mojang, intermediary or obfuscated names):
 * <pre>
 *   constructors:  this.threadingDetector = new ThreadingDetector("PalettedContainer");
 *   acquire():     this.threadingDetector.checkAndLock();
 *   release():     this.threadingDetector.checkAndUnlock();
 * </pre>
 * If anything else in the class reads the field (another mod's code merged into it, for example), or the guard is
 * already gone (FerriteCore), the class is left alone.
 */
final class SlimSections extends Patch {
    static final String NAME = "PalettedContainer";

    SlimSections() {
        super("slim-sections", 1, "no debug thread guard in each chunk section container (10-40 MB less memory)",
                new String[][] {{NAME, "(Ljava/lang/String;)V"}}, new String[] {"ferritecore"},
                "net/minecraft/world/level/chunk/PalettedContainer", "net/minecraft/class_2841");
    }

    @Override
    void apply(Target t, Ctx ctx) throws Skip {
        List<MethodNode> inits = t.methodsNamed("<init>");
        String guard = null;
        String field = null;
        List<AbstractInsnNode[]> sites = new ArrayList<AbstractInsnNode[]>();
        List<MethodNode> siteMethods = new ArrayList<MethodNode>();
        for (MethodNode m : inits) {
            for (AbstractInsnNode n = m.instructions.getFirst(); n != null; n = n.getNext()) {
                if (!(n instanceof LdcInsnNode) || !NAME.equals(((LdcInsnNode) n).cst)) {
                    continue;
                }
                AbstractInsnNode dup = Code.prev(n);
                AbstractInsnNode nw = dup == null ? null : Code.prev(dup);
                AbstractInsnNode init = Code.next(n);
                AbstractInsnNode put = init == null ? null : Code.next(init);
                if (!(nw instanceof TypeInsnNode) || nw.getOpcode() != Opcodes.NEW || dup.getOpcode() != Opcodes.DUP
                        || !(init instanceof MethodInsnNode) || !(put instanceof FieldInsnNode)
                        || put.getOpcode() != Opcodes.PUTFIELD) {
                    throw Skip.because("the guard is not created the expected way");
                }
                String type = ((TypeInsnNode) nw).desc;
                MethodInsnNode ci = (MethodInsnNode) init;
                FieldInsnNode fi = (FieldInsnNode) put;
                if (ci.getOpcode() != Opcodes.INVOKESPECIAL || !ci.owner.equals(type) || !ci.name.equals("<init>")
                        || !ci.desc.equals("(Ljava/lang/String;)V") || !fi.owner.equals(t.name)
                        || !fi.desc.equals("L" + type + ";") || (guard != null && !guard.equals(type))
                        || (field != null && !field.equals(fi.name))) {
                    throw Skip.because("the guard is not created the expected way");
                }
                guard = type;
                field = fi.name;
                sites.add(new AbstractInsnNode[] {nw, dup, n, init});
                if (!siteMethods.contains(m)) {
                    siteMethods.add(m);
                }
            }
        }
        if (sites.isEmpty()) {
            throw Skip.notTarget("no guard created here");
        }
        if (sites.size() > 8) {
            throw Skip.because("more places than expected");
        }
        int access = t.fieldAccess(field, "L" + guard + ";");
        if (access < 0 || (access & Opcodes.ACC_PRIVATE) == 0 || (access & Opcodes.ACC_STATIC) != 0) {
            throw Skip.because("the guard field is not a private field of this class");
        }

        // every other use of the field must be acquire() or release(): this.field.someMethod(); return;
        List<MethodNode> users = t.fieldUsers(t.name, field, "L" + guard + ";");
        List<MethodNode> calls = new ArrayList<MethodNode>();
        for (MethodNode m : users) {
            if (siteMethods.contains(m)) {
                // a constructor: only the putfields we replace may touch the field
                int puts = 0;
                for (AbstractInsnNode n = m.instructions.getFirst(); n != null; n = n.getNext()) {
                    if (n instanceof FieldInsnNode && ((FieldInsnNode) n).name.equals(field)
                            && ((FieldInsnNode) n).owner.equals(t.name)) {
                        puts++;
                    }
                }
                int mine = 0;
                for (AbstractInsnNode[] s : sites) {
                    mine += m.instructions.contains(s[0]) ? 1 : 0;
                }
                if (puts != mine) {
                    throw Skip.because("a constructor uses the guard in another way");
                }
                continue;
            }
            if (!isGuardCall(m, t.name, field, guard)) {
                throw Skip.because("the guard is used by " + m.name + m.desc);
            }
            calls.add(m);
        }
        // two: acquire() and release(); none: a mod (Lithium's no_locking) already made them stop using the guard
        if (calls.size() != 2 && calls.size() != 0) {
            throw Skip.because("expected acquire() and release(), found " + calls.size() + " methods using the guard");
        }

        for (AbstractInsnNode[] s : sites) {
            MethodNode m = owner(siteMethods, s[0]);
            m.instructions.insertBefore(s[0], new InsnNode(Opcodes.ACONST_NULL));
            for (AbstractInsnNode n : s) {
                m.instructions.remove(n);
            }
        }
        for (MethodNode m : siteMethods) {
            t.changed(m);
        }
        for (MethodNode m : calls) {
            InsnList body = new InsnList();
            body.add(new InsnNode(Opcodes.RETURN));
            m.instructions = body;
            m.tryCatchBlocks.clear();
            m.localVariables = null;
            m.visibleLocalVariableAnnotations = null;
            m.invisibleLocalVariableAnnotations = null;
            m.maxStack = 0;
            t.changed(m);
        }
    }

    /** this.field.method()V; return; - and nothing else (labels and line numbers aside). */
    private static boolean isGuardCall(MethodNode m, String owner, String field, String guard) {
        if ((m.access & Opcodes.ACC_STATIC) != 0 || !m.desc.equals("()V") || !m.tryCatchBlocks.isEmpty()
                || Code.changedByMixin(m)) {
            return false;
        }
        AbstractInsnNode self = Code.first(m);
        AbstractInsnNode get = self == null ? null : Code.next(self);
        AbstractInsnNode call = get == null ? null : Code.next(get);
        AbstractInsnNode ret = call == null ? null : Code.next(call);
        return Code.isVar(self, Opcodes.ALOAD, 0) && get instanceof FieldInsnNode
                && get.getOpcode() == Opcodes.GETFIELD && ((FieldInsnNode) get).owner.equals(owner)
                && ((FieldInsnNode) get).name.equals(field)
                && Code.isCall(call, Opcodes.INVOKEVIRTUAL, guard, null, "()V")
                && ret != null && ret.getOpcode() == Opcodes.RETURN && Code.next(ret) == null;
    }

    private static MethodNode owner(List<MethodNode> methods, AbstractInsnNode n) {
        for (MethodNode m : methods) {
            if (m.instructions.contains(n)) {
                return m;
            }
        }
        throw new IllegalStateException("instruction without method");
    }
}
