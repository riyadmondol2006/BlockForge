package com.blockforge.lite.core;

import java.util.List;
import org.objectweb.asm.Opcodes;
import org.objectweb.asm.tree.AbstractInsnNode;
import org.objectweb.asm.tree.FieldInsnNode;
import org.objectweb.asm.tree.MethodInsnNode;
import org.objectweb.asm.tree.MethodNode;
import org.objectweb.asm.tree.TypeInsnNode;

/**
 * section-lookup: UberGpuBuffer (the shared vertex/index buffer of the chunk sections, 26.3) keeps the live
 * allocation of every section mesh in a java.util.HashMap, and the render thread looks it up per section and
 * layer on every frame (getAllocation, from SectionRenderDispatcher.getRenderSectionSlice). In the profile
 * behind this patch that was HashMap.getNode all over the render thread.
 *
 * The patch makes only the constructor build the map differently:
 * <pre>
 *   this.allocationMap = new HashMap(256);   becomes   new Object2ObjectOpenHashMap(256);
 * </pre>
 * That is fastutil's open-addressing map: one array probe instead of a node chain, and no node object per
 * entry. The game already ships fastutil and this same class already keeps its staged allocations in an
 * Object2ObjectOpenHashMap on the same threads, so no new library and no new thread-safety story is added
 * (neither map is safe for unsynchronised sharing; the class is used as before).
 *
 * Why not Reference2ObjectOpenHashMap (identity keys, the fastest of the three): UberGpuBuffer is a generic
 * class, and an identity map would change what an equal-but-different key finds. The one key type of 26.3,
 * SectionMesh (CompiledSectionMesh and its two anonymous constants), defines no equals/hashCode, so identity
 * would work here, but Object2Object keeps HashMap's exact key meaning for every key, so the patch does not
 * have to trust that. It also keeps the rest of the Map contract the class uses: a null key finds nothing
 * (and may be stored), an absent key reads and removes as null, which is fastutil's default return value.
 * Checked on the 26.3 bytecode: allocationMap is used in exactly four places, all through the Map interface
 * (put when an upload lands, get in getAllocation, remove in freeAllocation, clear in close); it is never
 * iterated, returned or handed out, so iteration order and fail-fast behaviour cannot be seen. Values put
 * are the non-null allocations just made; keys are the meshes themselves. Anything shaped differently
 * (another use of the map, a mod's change to one of those methods, another starting map) is left alone.
 *
 * Sodium and Iris replace this whole rendering path and VulkanMod replaces the renderer, so the patch stays
 * off with them.
 */
final class SectionLookup extends Patch {
    static final String HASH_MAP = "java/util/HashMap";
    static final String FAST_MAP = "it/unimi/dsi/fastutil/objects/Object2ObjectOpenHashMap";
    private static final String MAP = "java/util/Map";
    private static final String FIELD = "allocationMap";
    private static final String FIELD_DESC = "Ljava/util/Map;";

    SectionLookup() {
        super("section-lookup", 1, "section drawing finds each section's place in the shared GPU buffer in a "
                + "faster map (less work on the render thread every frame; Minecraft 26.3)",
                new String[][] {{FIELD, HASH_MAP, FAST_MAP}},
                new String[] {"sodium", "iris", "vulkanmod"},
                "com/mojang/blaze3d/vertex/UberGpuBuffer");
    }

    @Override
    void apply(Target t, Ctx ctx) throws Skip {
        int access = t.fieldAccess(FIELD, FIELD_DESC);
        if (access < 0) {
            throw Skip.notTarget("no allocationMap");
        }
        if ((access & Opcodes.ACC_PRIVATE) == 0 || (access & Opcodes.ACC_FINAL) == 0
                || (access & Opcodes.ACC_STATIC) != 0) {
            throw Skip.because("allocationMap is not a private final field");
        }

        // every use of the field, and every HashMap made in the class, have to be the ones checked in 26.3
        MethodNode ctor = null;
        AbstractInsnNode made = null;
        AbstractInsnNode hashCtor = null;
        int puts = 0;
        int gets = 0;
        int mapCalls = 0;
        int hashNews = 0;
        int hashCtors = 0;
        boolean fastInCtor = false;
        java.util.Map<String, Integer> byName = new java.util.HashMap<String, Integer>();
        List<MethodNode> all = t.allMethods();
        for (MethodNode m : all) {
            if (m.instructions == null) {
                continue;
            }
            boolean usesMap = false;
            for (AbstractInsnNode n = m.instructions.getFirst(); n != null; n = n.getNext()) {
                if (n instanceof TypeInsnNode && n.getOpcode() == Opcodes.NEW) {
                    String desc = ((TypeInsnNode) n).desc;
                    if (desc.equals(HASH_MAP)) {
                        hashNews++;
                    }
                    if (desc.equals(FAST_MAP) && m.name.equals("<init>")) {
                        fastInCtor = true;
                    }
                }
                if (n instanceof MethodInsnNode) {
                    MethodInsnNode call = (MethodInsnNode) n;
                    if (call.owner.equals(HASH_MAP) && call.name.equals("<init>")) {
                        hashCtors++;
                    }
                    if (call.owner.equals(MAP) && (call.getOpcode() == Opcodes.INVOKEINTERFACE
                            || call.getOpcode() == Opcodes.INVOKEVIRTUAL)) {
                        mapCalls++;
                        Integer c = byName.get(call.name);
                        byName.put(call.name, c == null ? 1 : c + 1);
                    }
                }
                if (!(n instanceof FieldInsnNode) || !((FieldInsnNode) n).owner.equals(t.name)
                        || !((FieldInsnNode) n).name.equals(FIELD) || !((FieldInsnNode) n).desc.equals(FIELD_DESC)) {
                    continue;
                }
                usesMap = true;
                if (n.getOpcode() == Opcodes.PUTFIELD) {
                    puts++;
                    if (!m.name.equals("<init>") || ctor != null) {
                        throw Skip.because("allocationMap is set outside the constructor, or more than once");
                    }
                    // this.allocationMap = new HashMap(256): the instructions just before the store
                    AbstractInsnNode[] before = Code.before(n, Opcodes.INVOKESPECIAL, Opcodes.SIPUSH, Opcodes.DUP,
                            Opcodes.NEW);
                    if (before == null || !Code.isCall(before[0], Opcodes.INVOKESPECIAL, HASH_MAP, "<init>", "(I)V")
                            || !Code.isInt(before[1], 256) || !((TypeInsnNode) before[3]).desc.equals(HASH_MAP)) {
                        throw Skip.because("allocationMap is not made as new HashMap(256)");
                    }
                    ctor = m;
                    hashCtor = before[0];
                    made = before[3];
                } else if (n.getOpcode() == Opcodes.GETFIELD) {
                    gets++;
                    if (mapCallAfter(n) == null) {
                        throw Skip.because("allocationMap is used in another way (" + m.name + m.desc + ")");
                    }
                } else {
                    throw Skip.because("allocationMap is used in another way (" + m.name + m.desc + ")");
                }
            }
            if (usesMap && Code.changedByMixin(m)) {
                throw Skip.because(m.name + " is changed by a mod");
            }
        }
        if (ctor == null) {
            throw Skip.because("allocationMap is not made in the constructor");
        }
        if (hashNews != 1 || hashCtors != 1) {
            throw Skip.because("the class makes a HashMap in another place");
        }
        if (!fastInCtor) {
            throw Skip.because("the constructor does not already use fastutil (stagedAllocations)");
        }
        if (gets != 4 || mapCalls != 4 || count(byName, "get") != 1 || count(byName, "put") != 1
                || count(byName, "remove") != 1 || count(byName, "clear") != 1) {
            throw Skip.because("allocationMap is used " + gets + " times (get/put/remove/clear expected once each)");
        }

        ((TypeInsnNode) made).desc = FAST_MAP;
        ((MethodInsnNode) hashCtor).owner = FAST_MAP;
        t.changed(ctor);
    }

    private static int count(java.util.Map<String, Integer> byName, String name) {
        Integer c = byName.get(name);
        return c == null ? 0 : c;
    }

    /**
     * The Map call a load of the field feeds, looking a few instructions ahead (an entry's getKey may come
     * first); null when the loaded map is stored, returned, checked or used by any other call first.
     */
    private static String mapCallAfter(AbstractInsnNode load) {
        AbstractInsnNode n = load;
        for (int i = 0; i < 6; i++) {
            n = Code.next(n);
            if (n == null) {
                return null;
            }
            if (n instanceof MethodInsnNode) {
                MethodInsnNode call = (MethodInsnNode) n;
                if (call.owner.equals(MAP) && (call.name.equals("get") || call.name.equals("put")
                        || call.name.equals("remove") || call.name.equals("clear"))) {
                    return call.name;
                }
                if (call.owner.equals("java/util/Map$Entry")) {
                    continue;
                }
                return null;
            }
            int op = n.getOpcode();
            if (op == Opcodes.ASTORE || op == Opcodes.ARETURN || op == Opcodes.PUTFIELD || op == Opcodes.PUTSTATIC
                    || op == Opcodes.CHECKCAST || op == Opcodes.INSTANCEOF || op == Opcodes.IFNULL
                    || op == Opcodes.IFNONNULL) {
                return null;
            }
        }
        return null;
    }
}
