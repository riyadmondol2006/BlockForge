package com.blockforge.lite.core;

import java.util.Map;
import org.objectweb.asm.Opcodes;
import org.objectweb.asm.tree.AbstractInsnNode;
import org.objectweb.asm.tree.InsnList;
import org.objectweb.asm.tree.LdcInsnNode;
import org.objectweb.asm.tree.MethodInsnNode;
import org.objectweb.asm.tree.MethodNode;
import org.objectweb.asm.tree.VarInsnNode;

/**
 * hidden-entities: the game draws every mob, item and other entity whose box is inside the field of view, even when it
 * is behind a hill or under the ground. For terrain the game already knows better: each frame it works out which chunk
 * sections can be seen from the camera (a search through the open space between sections) and draws only those.
 * The patch applies the same knowledge to entities: one whose box lies only in sections that search did not reach is
 * not drawn. Sodium does the same. Measured: hundreds of hidden mobs cost about a third of the render thread's Java
 * time.
 *
 * Never culled: players, the camera entity, riders and what they ride, named entities and glowing ones (their name or
 * outline shows through walls), entities on a lead, very large boxes, boxes reaching outside the world, and entities
 * touching a section without blocks (the game leaves those out of its list though they can be seen into). Only the
 * main view is affected, only after the game's own frustum test said yes, and any error means "drawn".
 *
 * Level 2 (not exact): the visibility search runs in the background after the camera moves into a new spot, so for a
 * frame or two an entity coming into view at the edge of a hill can appear one frame later; the terrain around it
 * does the same.
 *
 * Target, EntityRenderer.shouldRender in Minecraft 26.2 and newer (Mojang names on every loader):
 * <pre>
 *   AABB box = this.getBoundingBoxForCulling(entity, partialTicks).inflate(0.5);
 *   ...
 *   if (culler.isVisible(box)) return true;      // becomes: if (inView(culler.isVisible(box), entity, culler, box))
 * </pre>
 * Renderers that also draw outside the box (guardian and end crystal beams) ask this first and then test their beam,
 * so the beam still shows. The added code is a template ({@code tpl/HiddenEntitiesCode}); what it uses of the game
 * (Minecraft's level, level renderer and main camera, the visible section list, chunk sections) is checked before.
 */
final class HiddenEntities extends Patch {
    static final String RENDERER = "net/minecraft/client/renderer/entity/EntityRenderer";
    static final String FRUSTUM = "net/minecraft/client/renderer/culling/Frustum";
    static final String AABB = "net/minecraft/world/phys/AABB";
    static final String ENTITY = "net/minecraft/world/entity/Entity";

    HiddenEntities() {
        super("hidden-entities", 2, "mobs and items hidden behind terrain are not drawn (Minecraft 26.2 and newer)",
                new String[][] {{FRUSTUM, AABB, "inflate"}},
                new String[] {"sodium", "embeddium", "rubidium", "nvidium", "iris", "oculus", "optifine", "vulkanmod",
                    "entityculling"},
                RENDERER);
    }

    @Override
    void apply(Target t, Ctx ctx) throws Skip {
        MethodNode render = null;
        AbstractInsnNode test = null;
        for (Object[] site : t.calls(Opcodes.INVOKEVIRTUAL, FRUSTUM, "isVisible", "(L" + AABB + ";)Z")) {
            MethodNode m = (MethodNode) site[0];
            if ((m.access & Opcodes.ACC_STATIC) != 0 || !m.desc.endsWith(")Z")
                    || !m.desc.startsWith("(L" + ENTITY + ";L" + FRUSTUM + ";DDD")) {
                continue;
            }
            if (render != null && render != m) {
                throw Skip.because("two methods test entities against the frustum");
            }
            if (render == null) {
                render = m;
                test = (AbstractInsnNode) site[1];
            }
        }
        if (render == null) {
            throw Skip.notTarget("no entity frustum test here");
        }
        // the first test: culler.isVisible(box) with box = ....inflate(0.5), then if (...) return true
        AbstractInsnNode[] before = Code.before(test, Opcodes.ALOAD, Opcodes.ALOAD);
        AbstractInsnNode jump = Code.next(test);
        if (before == null || !Code.isVar(before[1], Opcodes.ALOAD, 2) || jump == null
                || jump.getOpcode() != Opcodes.IFEQ) {
            throw Skip.because("the frustum test looks different");
        }
        int box = ((VarInsnNode) before[0]).var;
        if (!inflatedInto(render, box)) {
            throw Skip.because("the entity box is made in an unexpected way");
        }
        if (Code.changedByMixin(render) || Code.callsModLoader(render)) {
            throw Skip.because("the frustum test was changed by a mod");
        }
        Map<String, MethodNode> code = Template.copy(t, ctx, "HiddenEntitiesCode");
        MethodNode inView = code.get(Code.PREFIX + "inView");
        String want = "(ZL" + ENTITY + ";L" + FRUSTUM + ";L" + AABB + ";)Z";
        if (inView == null || !inView.desc.equals(want)) {
            throw new IllegalStateException("template helper missing or with another descriptor");
        }
        InsnList call = new InsnList();
        call.add(new VarInsnNode(Opcodes.ALOAD, 1));
        call.add(new VarInsnNode(Opcodes.ALOAD, 2));
        call.add(new VarInsnNode(Opcodes.ALOAD, box));
        call.add(new MethodInsnNode(Opcodes.INVOKESTATIC, t.name, inView.name, inView.desc, false));
        render.instructions.insert(test, call);
        render.maxStack += 3;
        t.changed(render);
    }

    /** Whether the local is set from (something).inflate(0.5). */
    private static boolean inflatedInto(MethodNode m, int var) {
        for (AbstractInsnNode n = m.instructions.getFirst(); n != null; n = n.getNext()) {
            if (Code.isVar(n, Opcodes.ASTORE, var)) {
                AbstractInsnNode call = Code.prev(n);
                AbstractInsnNode half = call == null ? null : Code.prev(call);
                if (Code.isCall(call, Opcodes.INVOKEVIRTUAL, AABB, "inflate", "(D)L" + AABB + ";")
                        && half instanceof LdcInsnNode && Double.valueOf(0.5).equals(((LdcInsnNode) half).cst)) {
                    return true;
                }
            }
        }
        return false;
    }
}
