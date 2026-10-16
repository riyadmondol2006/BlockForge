package com.blockforge.lite.core;

import java.util.ArrayList;
import java.util.List;
import java.util.Map;
import org.objectweb.asm.ClassVisitor;
import org.objectweb.asm.ClassReader;
import org.objectweb.asm.Label;
import org.objectweb.asm.MethodVisitor;
import org.objectweb.asm.Opcodes;
import org.objectweb.asm.Type;
import org.objectweb.asm.tree.AbstractInsnNode;
import org.objectweb.asm.tree.InsnList;
import org.objectweb.asm.tree.InsnNode;
import org.objectweb.asm.tree.MethodInsnNode;
import org.objectweb.asm.tree.MethodNode;
import org.objectweb.asm.tree.TypeInsnNode;
import org.objectweb.asm.tree.VarInsnNode;

/**
 * tint-memo: the colour of grass, leaves and water is the average of the biomes around the block (5 x 5 columns with
 * the default "biome blend" of 2). Neighbouring blocks share 20 of their 25 samples, but each block looks all of them
 * up again, and each lookup is the expensive fuzzy biome search. While chunks are built (on the worker threads) this
 * was up to a quarter of the build time near water. The patch remembers the last few thousand looked-up biomes per
 * thread, so each one is looked up about once per chunk section; the colours are exactly the same.
 *
 * Target, ClientLevel in every version from 1.20 to 26.x:
 * <pre>
 *   int calculateBlockTint(BlockPos pos, ColorResolver resolver) {
 *       ...
 *       Cursor3D cursor = new Cursor3D(pos.getX() - r, pos.getY(), pos.getZ() - r, pos.getX() + r, ...);
 *       MutableBlockPos p = new MutableBlockPos();
 *       while (cursor.advance()) { p.set(...); int c = resolver.getColor(this.getBiome(p).value(), ...); ... }
 *   }
 *   onChunkLoaded(ChunkPos pos): ... tintCache.invalidateForChunk(pos.x, pos.z) ...
 *   clearTintCaches():           ... tintCache.invalidateAll() ...
 * </pre>
 * The getBiome call in the loop goes through a method the patch adds ({@link #helper}); the memory itself is the
 * template {@code tpl/TintMemoCode} (JDK only). It is per thread, holds the biomes of one level only, and forgets what
 * may have changed whenever the game clears its own colour cache: everything near a chunk that arrived or changed its
 * biomes (where the game clears that chunk's colours), everything on a resource reload.
 */
final class TintMemo extends Patch {
    TintMemo() {
        super("tint-memo", 1, "remembers biome lookups while the colours of grass, leaves and water are blended",
                new String[][] {{"ClientLevel", "(IIIIII)V"}}, new String[] {"optifine", "vulkanmod"},
                "net/minecraft/client/multiplayer/ClientLevel", "net/minecraft/class_638");
    }

    @Override
    void apply(Target t, Ctx ctx) throws Skip {
        if (t.isInterface()) {
            throw Skip.notTarget("an interface");
        }
        // 1. the blend method: (pos, resolver)I with a 6-int cursor, two biome lookups and two getColor calls
        List<String[]> twoArgInts = methods(t, "I", 2);
        MethodNode blend = null;
        Shape shape = null;
        for (String[] nd : twoArgInts) {
            MethodNode m = t.method(nd[0], nd[1]);
            Shape s = m == null ? null : shape(t, m);
            if (s != null) {
                if (blend != null) {
                    throw Skip.because("two methods blend biome colours");
                }
                blend = m;
                shape = s;
            }
        }
        if (blend == null) {
            throw Skip.notTarget("no biome blend here");
        }
        if (Code.changedByMixin(blend)) {
            throw Skip.because("the biome blend was changed by a mod");
        }

        // 2. the colour cache the game clears: getBlockTint's (TintCache) cast, then its (II)V and ()V calls
        String cache = null;
        for (String[] nd : twoArgInts) {
            MethodNode m = t.method(nd[0], nd[1]);
            if (m == null || m == blend || !m.desc.equals(blend.desc)) {
                continue;
            }
            // (TintCache) this.tintCaches.get(resolver) ... .getColor(pos)
            List<String> casts = new ArrayList<String>();
            for (AbstractInsnNode n = m.instructions.getFirst(); n != null; n = n.getNext()) {
                if (n.getOpcode() == Opcodes.CHECKCAST) {
                    casts.add(((TypeInsnNode) n).desc);
                }
            }
            for (AbstractInsnNode n = m.instructions.getFirst(); n != null; n = n.getNext()) {
                if (n instanceof MethodInsnNode && n.getOpcode() == Opcodes.INVOKEVIRTUAL
                        && casts.contains(((MethodInsnNode) n).owner)
                        && ((MethodInsnNode) n).desc.equals("(L" + shape.pos + ";)I")) {
                    if (cache != null && !cache.equals(((MethodInsnNode) n).owner)) {
                        throw Skip.because("two colour caches");
                    }
                    cache = ((MethodInsnNode) n).owner;
                }
            }
        }
        if (cache == null) {
            throw Skip.because("the colour cache was not found");
        }
        List<Object[]> clears = new ArrayList<Object[]>();
        clears.addAll(t.calls(Opcodes.INVOKEVIRTUAL, cache, null, "(II)V"));
        int perChunk = clears.size();
        clears.addAll(t.calls(Opcodes.INVOKEVIRTUAL, cache, null, "()V"));
        if (perChunk != 1 || clears.size() != 2) {
            throw Skip.because("the colour cache is cleared in an unexpected way (" + clears.size() + " places)");
        }
        for (Object[] site : clears) {
            if (Code.changedByMixin((MethodNode) site[0])) {
                throw Skip.because("the colour cache clearing was changed by a mod");
            }
        }

        // 3. change: the loop's lookup goes through the memo; the game's clearing of its colour cache tells the memo
        Map<String, MethodNode> code = Template.copy(t, ctx, "TintMemoCode");
        MethodNode helper = helper(t.name, shape, t.needsFrames());
        t.add(helper);
        blend.instructions.set(shape.loopCall,
                new MethodInsnNode(Opcodes.INVOKESTATIC, t.name, helper.name, helper.desc, false));
        t.changed(blend);
        MethodNode chunk = code.get(Code.PREFIX + "tintChunk");
        MethodNode clear = code.get(Code.PREFIX + "tintClear");
        for (Object[] site : clears) {
            MethodNode m = (MethodNode) site[0];
            MethodInsnNode call = (MethodInsnNode) site[1];
            InsnList tell = new InsnList();
            if (call.desc.equals("(II)V")) {
                // the chunk's x and z are on the stack for the call; the memo gets them too
                tell.add(new InsnNode(Opcodes.DUP2));
                tell.add(new MethodInsnNode(Opcodes.INVOKESTATIC, t.name, chunk.name, chunk.desc, false));
            } else {
                tell.add(new MethodInsnNode(Opcodes.INVOKESTATIC, t.name, clear.name, clear.desc, false));
            }
            m.instructions.insertBefore(call, tell);
            m.maxStack += 2;
            t.changed(m);
        }
    }

    /** What the patch needs to know about the blend method. */
    static final class Shape {
        String pos;
        /** getBiome(BlockPos) as called (owner, name, descriptor). */
        MethodInsnNode loopCall;
        /** pos.getX(), getY(), getZ(). */
        MethodInsnNode x;
        MethodInsnNode y;
        MethodInsnNode z;
    }

    /** name and descriptor of the non-static methods that take `args` objects and return `ret`. */
    static List<String[]> methods(Target t, final String ret, final int args) {
        final List<String[]> out = new ArrayList<String[]>();
        t.reader.accept(new ClassVisitor(Opcodes.ASM9) {
            @Override
            public MethodVisitor visitMethod(int a, String n, String d, String s, String[] e) {
                Type[] at = Type.getArgumentTypes(d);
                boolean objects = at.length == args;
                for (Type x : at) {
                    objects &= x.getSort() == Type.OBJECT;
                }
                if ((a & Opcodes.ACC_STATIC) == 0 && objects && Type.getReturnType(d).getDescriptor().equals(ret)) {
                    out.add(new String[] {n, d});
                }
                return null;
            }
        }, ClassReader.SKIP_CODE);
        return out;
    }

    /** The blend method's shape, or null when this method is not it. */
    static Shape shape(Target t, MethodNode m) {
        Type[] args = Type.getArgumentTypes(m.desc);
        String pos = args[0].getInternalName();
        String resolver = args[1].getInternalName();
        // two resolver.getColor(Biome, double, double) calls
        List<AbstractInsnNode> colors = Code.calls(m, Opcodes.INVOKEINTERFACE, resolver, null, null);
        if (colors.size() != 2) {
            return null;
        }
        String biome = null;
        for (AbstractInsnNode c : colors) {
            Type[] ca = Type.getArgumentTypes(((MethodInsnNode) c).desc);
            Type ret = Type.getReturnType(((MethodInsnNode) c).desc);
            if (ca.length != 3 || ca[0].getSort() != Type.OBJECT || ca[1] != Type.DOUBLE_TYPE
                    || ca[2] != Type.DOUBLE_TYPE || !ret.equals(Type.INT_TYPE)) {
                return null;
            }
            if (biome != null && !biome.equals(ca[0].getInternalName())) {
                return null;
            }
            biome = ca[0].getInternalName();
        }
        // two this.getBiome(pos).value() cast to the biome
        List<MethodInsnNode> lookups = new ArrayList<MethodInsnNode>();
        for (AbstractInsnNode n = m.instructions.getFirst(); n != null; n = n.getNext()) {
            if (!(n instanceof MethodInsnNode) || n.getOpcode() != Opcodes.INVOKEVIRTUAL) {
                continue;
            }
            MethodInsnNode c = (MethodInsnNode) n;
            if (!c.owner.equals(t.name) || !c.desc.startsWith("(L" + pos + ";)L")) {
                continue;
            }
            AbstractInsnNode value = Code.next(c);
            AbstractInsnNode cast = value == null ? null : Code.next(value);
            String holder = Type.getReturnType(c.desc).getInternalName();
            if (!Code.isCall(value, Opcodes.INVOKEINTERFACE, holder, null, "()Ljava/lang/Object;")
                    || !(cast instanceof TypeInsnNode) || cast.getOpcode() != Opcodes.CHECKCAST
                    || !((TypeInsnNode) cast).desc.equals(biome)) {
                return null;
            }
            lookups.add(c);
        }
        if (lookups.size() != 2 || !lookups.get(0).name.equals(lookups.get(1).name)
                || !lookups.get(0).desc.equals(lookups.get(1).desc)) {
            return null;
        }
        // the first lookup is for pos itself (blend radius 0), the second for the cursor's position in the loop
        AbstractInsnNode[] first = Code.before(lookups.get(0), Opcodes.ALOAD, Opcodes.ALOAD);
        AbstractInsnNode[] second = Code.before(lookups.get(1), Opcodes.ALOAD, Opcodes.ALOAD);
        if (first == null || second == null || !Code.isVar(first[0], Opcodes.ALOAD, 1)
                || !Code.isVar(first[1], Opcodes.ALOAD, 0) || !Code.isVar(second[1], Opcodes.ALOAD, 0)
                || ((VarInsnNode) second[0]).var <= 2) {
            return null;
        }
        // new Cursor3D(pos.getX() - r, pos.getY(), pos.getZ() - r, pos.getX() + r, pos.getY(), pos.getZ() + r)
        AbstractInsnNode init = null;
        for (AbstractInsnNode n = m.instructions.getFirst(); n != null; n = n.getNext()) {
            if (n.getOpcode() == Opcodes.INVOKESPECIAL && ((MethodInsnNode) n).name.equals("<init>")
                    && ((MethodInsnNode) n).desc.equals("(IIIIII)V")) {
                if (init != null) {
                    return null;
                }
                init = n;
            }
        }
        if (init == null) {
            return null;
        }
        List<MethodInsnNode> coords = new ArrayList<MethodInsnNode>();
        List<Boolean> shifted = new ArrayList<Boolean>();
        AbstractInsnNode n = Code.prev(init);
        for (int i = 0; i < 40 && n != null && n.getOpcode() != Opcodes.NEW; i++, n = Code.prev(n)) {
            if (n instanceof MethodInsnNode) {
                MethodInsnNode c = (MethodInsnNode) n;
                AbstractInsnNode after = Code.next(c);
                if (c.getOpcode() != Opcodes.INVOKEVIRTUAL || !c.desc.equals("()I") || !c.owner.equals(pos)
                        || !Code.isVar(Code.prev(c), Opcodes.ALOAD, 1)) {
                    return null;
                }
                coords.add(0, c);
                shifted.add(0, after != null && after.getOpcode() == Opcodes.ILOAD);
            }
        }
        if (n == null || n.getOpcode() != Opcodes.NEW || coords.size() != 6) {
            return null;
        }
        for (int i = 0; i < 3; i++) {
            if (!coords.get(i).name.equals(coords.get(i + 3).name) || shifted.get(i) != shifted.get(i + 3)) {
                return null;
            }
        }
        // x and z are shifted by the radius, y is not
        if (!shifted.get(0) || shifted.get(1) || !shifted.get(2) || coords.get(0).name.equals(coords.get(1).name)
                || coords.get(0).name.equals(coords.get(2).name) || coords.get(1).name.equals(coords.get(2).name)) {
            return null;
        }
        Shape s = new Shape();
        s.pos = pos;
        s.loopCall = lookups.get(1);
        s.x = coords.get(0);
        s.y = coords.get(1);
        s.z = coords.get(2);
        return s;
    }

    /**
     * <pre>
     * private static Holder blockforge$lite$biome(ClientLevel level, BlockPos pos) {
     *     int x = pos.getX(), y = pos.getY(), z = pos.getZ();
     *     Object biome = tintGet(level, x, y, z);
     *     if (biome == null) {
     *         biome = level.getBiome(pos);
     *         tintPut(x, y, z, biome);
     *     }
     *     return (Holder) biome;
     * }
     * </pre>
     */
    static MethodNode helper(String owner, Shape s, boolean frames) {
        MethodInsnNode get = s.loopCall;
        String holder = Type.getReturnType(get.desc).getInternalName();
        MethodNode h = new MethodNode(Opcodes.ACC_PRIVATE | Opcodes.ACC_STATIC | Opcodes.ACC_SYNTHETIC,
                Code.PREFIX + "biome", "(L" + owner + ";L" + s.pos + ";)L" + holder + ";", null, null);
        final int level = 0, pos = 1, x = 2, y = 3, z = 4, biome = 5;
        Label have = new Label();
        h.visitCode();
        coord(h, pos, s.x, x);
        coord(h, pos, s.y, y);
        coord(h, pos, s.z, z);
        h.visitVarInsn(Opcodes.ALOAD, level);
        h.visitVarInsn(Opcodes.ILOAD, x);
        h.visitVarInsn(Opcodes.ILOAD, y);
        h.visitVarInsn(Opcodes.ILOAD, z);
        h.visitMethodInsn(Opcodes.INVOKESTATIC, owner, Code.PREFIX + "tintGet",
                "(Ljava/lang/Object;III)Ljava/lang/Object;", false);
        h.visitVarInsn(Opcodes.ASTORE, biome);
        h.visitVarInsn(Opcodes.ALOAD, biome);
        h.visitJumpInsn(Opcodes.IFNONNULL, have);
        h.visitVarInsn(Opcodes.ALOAD, level);
        h.visitVarInsn(Opcodes.ALOAD, pos);
        h.visitMethodInsn(get.getOpcode(), get.owner, get.name, get.desc, get.itf);
        h.visitVarInsn(Opcodes.ASTORE, biome);
        h.visitVarInsn(Opcodes.ILOAD, x);
        h.visitVarInsn(Opcodes.ILOAD, y);
        h.visitVarInsn(Opcodes.ILOAD, z);
        h.visitVarInsn(Opcodes.ALOAD, biome);
        h.visitMethodInsn(Opcodes.INVOKESTATIC, owner, Code.PREFIX + "tintPut", "(IIILjava/lang/Object;)V", false);
        h.visitLabel(have);
        if (frames) {
            // the biome local is an Object on both ways in (a Holder is one too)
            h.visitFrame(Opcodes.F_FULL, 6, new Object[] {owner, s.pos, Opcodes.INTEGER, Opcodes.INTEGER,
                Opcodes.INTEGER, "java/lang/Object"}, 0, new Object[0]);
        }
        h.visitVarInsn(Opcodes.ALOAD, biome);
        h.visitTypeInsn(Opcodes.CHECKCAST, holder);
        h.visitInsn(Opcodes.ARETURN);
        h.visitMaxs(4, 6);
        h.visitEnd();
        return h;
    }

    private static void coord(MethodNode h, int pos, MethodInsnNode call, int var) {
        h.visitVarInsn(Opcodes.ALOAD, pos);
        h.visitMethodInsn(Opcodes.INVOKEVIRTUAL, call.owner, call.name, call.desc, call.itf);
        h.visitVarInsn(Opcodes.ISTORE, var);
    }
}
