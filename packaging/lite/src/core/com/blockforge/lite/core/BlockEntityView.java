package com.blockforge.lite.core;

import java.util.List;
import java.util.Map;
import org.objectweb.asm.Opcodes;
import org.objectweb.asm.tree.AbstractInsnNode;
import org.objectweb.asm.tree.FieldInsnNode;
import org.objectweb.asm.tree.InsnList;
import org.objectweb.asm.tree.MethodInsnNode;
import org.objectweb.asm.tree.MethodNode;
import org.objectweb.asm.tree.VarInsnNode;

/**
 * block-entity-view: chests, signs, banners, shulker boxes and the other block entities are drawn for every chunk
 * section the game draws, and the game tests whole sections against a field of view that it moves 8 blocks back to be
 * safe. So in a storage room or a street of signs, many of those drawn are beside or behind the camera. The patch
 * tests each one against the field of view as well (a box one block around it, two above), as EntityCulling does.
 *
 * Only for the game's own renderers whose drawing stays inside that box (chest, shulker box, banner, signs, decorated
 * pot, campfire, lectern, bell, enchanting table, vault, brushable blocks, shelf, copper golem statue); not for
 * beacons and end gateways (beams), structure and test blocks (large boxes), heads (they load skins when first
 * drawn), spawners, conduits, pistons, or any renderer of a mod. The picture is the same.
 *
 * Target, BlockEntityRenderDispatcher.tryExtractRenderState in Minecraft 26.2 and newer:
 * <pre>
 *   if (!renderer.shouldRender(blockEntity, this.cameraPos)) return null;
 *   // becomes: if (!beInView(renderer.shouldRender(blockEntity, this.cameraPos), blockEntity, renderer)) return null;
 * </pre>
 * The added code is the template {@code tpl/BlockEntityViewCode}.
 */
final class BlockEntityView extends Patch {
    static final String DISPATCHER = "net/minecraft/client/renderer/blockentity/BlockEntityRenderDispatcher";
    static final String RENDERER = "net/minecraft/client/renderer/blockentity/BlockEntityRenderer";
    static final String BLOCK_ENTITY = "net/minecraft/world/level/block/entity/BlockEntity";
    static final String SHOULD = "(L" + BLOCK_ENTITY + ";Lnet/minecraft/world/phys/Vec3;)Z";

    BlockEntityView() {
        super("block-entity-view", 1, "chests, signs and banners outside the field of view are not drawn (26.2+)",
                new String[][] {{RENDERER, "shouldRender"}},
                new String[] {"entityculling", "moreculling", "sodium", "embeddium", "iris", "oculus", "optifine",
                    "vulkanmod"}, DISPATCHER);
    }

    @Override
    void apply(Target t, Ctx ctx) throws Skip {
        List<Object[]> sites = t.calls(Opcodes.INVOKEINTERFACE, RENDERER, "shouldRender", SHOULD);
        if (sites.isEmpty()) {
            throw Skip.notTarget("no block entity distance test here");
        }
        if (sites.size() != 1) {
            throw Skip.because("the distance test is made in " + sites.size() + " places");
        }
        MethodNode m = (MethodNode) sites.get(0)[0];
        AbstractInsnNode call = (AbstractInsnNode) sites.get(0)[1];
        // renderer.shouldRender(blockEntity, this.cameraPos), then a jump on the result
        AbstractInsnNode[] before = Code.before(call, Opcodes.GETFIELD, Opcodes.ALOAD, Opcodes.ALOAD, Opcodes.ALOAD);
        AbstractInsnNode jump = Code.next(call);
        if (before == null || !Code.isVar(before[1], Opcodes.ALOAD, 0) || !Code.isVar(before[2], Opcodes.ALOAD, 1)
                || !((FieldInsnNode) before[0]).owner.equals(t.name) || jump == null
                || (jump.getOpcode() != Opcodes.IFNE && jump.getOpcode() != Opcodes.IFEQ)
                || (m.access & Opcodes.ACC_STATIC) != 0 || !m.desc.startsWith("(L" + BLOCK_ENTITY + ";")) {
            throw Skip.because("the distance test looks different");
        }
        int renderer = ((VarInsnNode) before[3]).var;
        if (Code.changedByMixin(m) || Code.callsModLoader(m)) {
            throw Skip.because("the block entity code was changed by a mod");
        }
        Map<String, MethodNode> code = Template.copy(t, ctx, "BlockEntityViewCode");
        MethodNode inView = code.get(Code.PREFIX + "beInView");
        InsnList add = new InsnList();
        add.add(new VarInsnNode(Opcodes.ALOAD, 1));
        add.add(new VarInsnNode(Opcodes.ALOAD, renderer));
        add.add(new MethodInsnNode(Opcodes.INVOKESTATIC, t.name, inView.name, inView.desc, false));
        m.instructions.insert(call, add);
        m.maxStack += 2;
        t.changed(m);
    }
}
