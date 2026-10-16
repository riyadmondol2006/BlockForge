package com.blockforge.lite.core;

import org.objectweb.asm.Label;
import org.objectweb.asm.Opcodes;
import org.objectweb.asm.Type;
import org.objectweb.asm.tree.AbstractInsnNode;
import org.objectweb.asm.tree.FieldInsnNode;
import org.objectweb.asm.tree.InsnNode;
import org.objectweb.asm.tree.MethodInsnNode;
import org.objectweb.asm.tree.MethodNode;
import org.objectweb.asm.tree.VarInsnNode;

/**
 * lazy-collision-shape: every collision query of an entity (moving, "is there room here", items, particles, on the
 * server and the client) walks the blocks around the entity with a BlockCollisions iterator, and its constructor
 * turns the entity's box into a VoxelShape (Shapes.create: the shape plus three coordinate lists and their arrays,
 * about 200 bytes). That shape is only needed when the walk meets a block with a partial shape (a slab, stairs, a
 * fence...); most queries never get there.
 *
 * The patch makes the constructor store null and creates the very same shape the first time the iterator needs it:
 * <pre>
 *   constructor:    this.entityShape = Shapes.create(box);   becomes  this.entityShape = null;
 *   computeNext():  ... this.entityShape ...                becomes  ... blockforge$lite$shape(this) ...
 *   added:          static VoxelShape blockforge$lite$shape(BlockCollisions c) {
 *                       if (c.entityShape == null) c.entityShape = Shapes.create(c.box);
 *                       return c.entityShape; }
 * </pre>
 * Shapes.create only reads the box (an immutable AABB that the iterator also keeps), and the iterator is used by one
 * thread, so the shape is the same value, only made later or not at all. The field loses its final flag.
 *
 * Applies where the shape is exactly this: 1.17 - 26.x (checked 1.20.1 - 26.4-snapshot-4). Left alone when another
 * method reads or writes the field (a mod's accessor or merged code), or a mod changed computeNext.
 */
final class LazyCollisionShape extends Patch {
    static final String HELPER = Code.PREFIX + "shape";
    static final String ITERATOR = "com/google/common/collect/AbstractIterator";

    LazyCollisionShape() {
        super("lazy-collision-shape", 1, "collision checks make the entity's shape only when a partial block needs it "
                + "(less garbage on the server and client threads)",
                new String[][] {{ITERATOR, "computeNext", "endOfData"}}, new String[0],
                "net/minecraft/world/level/BlockCollisions", "net/minecraft/class_5329");
    }

    @Override
    void apply(Target t, Ctx ctx) throws Skip {
        if (!ITERATOR.equals(t.reader.getSuperName())) {
            throw Skip.notTarget("not an AbstractIterator");
        }
        // constructor: aload N (the box); invokestatic S.create(AABB)VoxelShape; putfield this.shape
        MethodNode init = null;
        MethodInsnNode create = null;
        FieldInsnNode put = null;
        int boxVar = -1;
        boolean twice = false;
        for (MethodNode m : t.methodsNamed("<init>")) {
            for (AbstractInsnNode n = m.instructions.getFirst(); n != null; n = n.getNext()) {
                if (!(n instanceof MethodInsnNode) || n.getOpcode() != Opcodes.INVOKESTATIC) {
                    continue;
                }
                MethodInsnNode c = (MethodInsnNode) n;
                Type[] args = Type.getArgumentTypes(c.desc);
                AbstractInsnNode load = Code.prev(c);
                AbstractInsnNode next = Code.next(c);
                if (args.length != 1 || args[0].getSort() != Type.OBJECT || c.itf
                        || Type.getReturnType(c.desc).getSort() != Type.OBJECT || !(load instanceof VarInsnNode)
                        || load.getOpcode() != Opcodes.ALOAD || !Code.isVar(Code.prev(load), Opcodes.ALOAD, 0)
                        || !(next instanceof FieldInsnNode) || next.getOpcode() != Opcodes.PUTFIELD
                        || !((FieldInsnNode) next).owner.equals(t.name)
                        || !((FieldInsnNode) next).desc.equals(Type.getReturnType(c.desc).getDescriptor())
                        || !keptInField(m, t.name, ((VarInsnNode) load).var, args[0].getDescriptor())) {
                    continue;
                }
                if (create != null) {
                    twice = true;
                    continue;
                }
                init = m;
                create = c;
                put = (FieldInsnNode) next;
                boxVar = ((VarInsnNode) load).var;
            }
        }
        if (create == null) {
            throw Skip.notTarget("no shape made from the box");
        }
        String shapeDesc = put.desc;
        // what makes this BlockCollisions (so the static call is Shapes.create, which only reads the box):
        // computeNext compares with Shapes.block() and passes the field to Shapes.joinIsNotEmpty(a, b, BooleanOp.AND)
        MethodNode next = t.method("computeNext", "()Ljava/lang/Object;");
        if (next == null || joinRead(next, t.name, create.owner, shapeDesc) == null
                || Code.calls(next, Opcodes.INVOKESTATIC, create.owner, null, "()" + shapeDesc).isEmpty()) {
            throw Skip.notTarget("not the collision iterator");
        }
        if (twice) {
            throw Skip.because("the shape is made in more than one place");
        }
        String boxDesc = Type.getArgumentTypes(create.desc)[0].getDescriptor();
        String shapeField = put.name;

        // the box is kept in a final field set from the same local, which the constructor never changes
        String boxField = null;
        for (AbstractInsnNode n = init.instructions.getFirst(); n != null; n = n.getNext()) {
            if (n instanceof VarInsnNode && n.getOpcode() == Opcodes.ASTORE && ((VarInsnNode) n).var == boxVar) {
                throw Skip.because("the constructor changes the box");
            }
            if (n instanceof FieldInsnNode && n.getOpcode() == Opcodes.PUTFIELD && ((FieldInsnNode) n).owner.equals(t.name)
                    && ((FieldInsnNode) n).desc.equals(boxDesc) && Code.isVar(Code.prev(n), Opcodes.ALOAD, boxVar)) {
                if (boxField != null) {
                    throw Skip.because("the box is kept twice");
                }
                boxField = ((FieldInsnNode) n).name;
            }
        }
        int boxAccess = boxField == null ? -1 : t.fieldAccess(boxField, boxDesc);
        int shapeAccess = t.fieldAccess(shapeField, shapeDesc);
        if (boxAccess < 0 || (boxAccess & Opcodes.ACC_FINAL) == 0 || (boxAccess & Opcodes.ACC_STATIC) != 0) {
            throw Skip.because("the box is not kept in a final field");
        }
        if (shapeAccess < 0 || (shapeAccess & Opcodes.ACC_PRIVATE) == 0 || (shapeAccess & Opcodes.ACC_STATIC) != 0) {
            throw Skip.because("the shape field is not a private field of this class");
        }

        // the shape is read once, by computeNext, and set only by that constructor
        MethodNode reader = null;
        FieldInsnNode get = null;
        for (MethodNode m : t.fieldUsers(t.name, shapeField, shapeDesc)) {
            for (AbstractInsnNode n = m.instructions.getFirst(); n != null; n = n.getNext()) {
                if (!(n instanceof FieldInsnNode) || !((FieldInsnNode) n).owner.equals(t.name)
                        || !((FieldInsnNode) n).name.equals(shapeField)) {
                    continue;
                }
                if (n == put) {
                    continue;
                }
                if (n.getOpcode() != Opcodes.GETFIELD || get != null) {
                    throw Skip.because("the shape is used in another way (" + m.name + m.desc + ")");
                }
                reader = m;
                get = (FieldInsnNode) n;
            }
        }
        if (get == null || reader != next || get != joinRead(next, t.name, create.owner, shapeDesc)) {
            throw Skip.because("the shape is not read the expected way");
        }
        if (Code.changedByMixin(reader) || Code.changedByMixin(init)) {
            throw Skip.because("the collision code is changed by a mod");
        }

        t.unfinal(shapeField, shapeDesc);
        MethodNode helper = helper(t.name, shapeField, shapeDesc, boxField, boxDesc, create, t.needsFrames());
        t.add(helper);
        init.instructions.insertBefore(create, new InsnNode(Opcodes.POP));
        init.instructions.set(create, new InsnNode(Opcodes.ACONST_NULL));
        reader.instructions.set(get, new MethodInsnNode(Opcodes.INVOKESTATIC, t.name, helper.name, helper.desc, false));
        t.changed(init);
        t.changed(reader);
    }

    /**
     * The read of a shape field that goes straight into the join: aload_0; getfield this.f; getstatic (BooleanOp.AND);
     * invokestatic Shapes.joinIsNotEmpty(VoxelShape, VoxelShape, ..)Z. Null when there is none or more than one.
     */
    private static FieldInsnNode joinRead(MethodNode m, String owner, String shapes, String shapeDesc) {
        FieldInsnNode found = null;
        for (AbstractInsnNode n = m.instructions.getFirst(); n != null; n = n.getNext()) {
            if (!(n instanceof FieldInsnNode) || n.getOpcode() != Opcodes.GETFIELD
                    || !((FieldInsnNode) n).owner.equals(owner) || !((FieldInsnNode) n).desc.equals(shapeDesc)
                    || !Code.isVar(Code.prev(n), Opcodes.ALOAD, 0)) {
                continue;
            }
            AbstractInsnNode op = Code.next(n);
            AbstractInsnNode join = op == null ? null : Code.next(op);
            if (op != null && op.getOpcode() == Opcodes.GETSTATIC && Code.isCall(join, Opcodes.INVOKESTATIC, shapes,
                    null, null) && ((MethodInsnNode) join).desc.startsWith("(" + shapeDesc + shapeDesc)
                    && ((MethodInsnNode) join).desc.endsWith(")Z")) {
                if (found != null) {
                    return null;
                }
                found = (FieldInsnNode) n;
            }
        }
        return found;
    }

    /** Whether the constructor stores this local (the box) straight into a field of its type: this.f = local. */
    private static boolean keptInField(MethodNode m, String owner, int var, String desc) {
        for (AbstractInsnNode n = m.instructions.getFirst(); n != null; n = n.getNext()) {
            if (n instanceof FieldInsnNode && n.getOpcode() == Opcodes.PUTFIELD && ((FieldInsnNode) n).owner.equals(owner)
                    && ((FieldInsnNode) n).desc.equals(desc) && Code.isVar(Code.prev(n), Opcodes.ALOAD, var)) {
                return true;
            }
        }
        return false;
    }

    /** static VoxelShape blockforge$lite$shape(BlockCollisions c): the shape, made from c's box on first use. */
    static MethodNode helper(String owner, String shapeField, String shapeDesc, String boxField, String boxDesc,
            MethodInsnNode create, boolean frames) {
        MethodNode h = new MethodNode(Opcodes.ACC_PRIVATE | Opcodes.ACC_STATIC | Opcodes.ACC_SYNTHETIC, HELPER,
                "(L" + owner + ";)" + shapeDesc, null, null);
        Label done = new Label();
        h.visitCode();
        h.visitVarInsn(Opcodes.ALOAD, 0);
        h.visitFieldInsn(Opcodes.GETFIELD, owner, shapeField, shapeDesc);
        h.visitInsn(Opcodes.DUP);
        h.visitJumpInsn(Opcodes.IFNONNULL, done);
        h.visitInsn(Opcodes.POP);
        h.visitVarInsn(Opcodes.ALOAD, 0);
        h.visitVarInsn(Opcodes.ALOAD, 0);
        h.visitFieldInsn(Opcodes.GETFIELD, owner, boxField, boxDesc);
        h.visitMethodInsn(Opcodes.INVOKESTATIC, create.owner, create.name, create.desc, create.itf);
        h.visitInsn(Opcodes.DUP_X1);
        h.visitFieldInsn(Opcodes.PUTFIELD, owner, shapeField, shapeDesc);
        h.visitLabel(done);
        if (frames) {
            h.visitFrame(Opcodes.F_SAME1, 0, null, 1, new Object[] {Type.getType(shapeDesc).getInternalName()});
        }
        h.visitInsn(Opcodes.ARETURN);
        h.visitMaxs(3, 1);
        h.visitEnd();
        return h;
    }
}
