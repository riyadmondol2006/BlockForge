package com.blockforge.lite.core;

import java.util.List;
import org.objectweb.asm.Label;
import org.objectweb.asm.Opcodes;
import org.objectweb.asm.tree.AbstractInsnNode;
import org.objectweb.asm.tree.FieldInsnNode;
import org.objectweb.asm.tree.FieldNode;
import org.objectweb.asm.tree.MethodInsnNode;
import org.objectweb.asm.tree.MethodNode;

/**
 * shared-state-tables: in Minecraft 1.20 and 1.21.1 every block state (about 27,000 in vanilla, many more with mods)
 * keeps a Guava ArrayTable of its neighbour states (property, value -> state). Each table builds its own row and
 * column index maps, although all states of a block have the same properties: about 16 MB of the heap in vanilla
 * 1.21.1 (measured by the analysis, A2 #10). Newer versions use plain arrays instead.
 *
 * The patch builds each table from a shared, empty template, so the index maps exist once per property list:
 * <pre>
 *   populateNeighbours(..):  this.neighbours = table.isEmpty() ? table : ArrayTable.create(table);
 *                  becomes   this.neighbours = table.isEmpty() ? table : blockforge$lite$table(table);
 *   added:  blockforge$lite$table(t): share(t), or the game's own ArrayTable.create(t) if that throws
 *           static ArrayTable blockforge$lite$share(Table t) {
 *               List rows = new ArrayList(t.rowKeySet());               // the properties, in the same order
 *               ArrayTable template = tables.get(rows);
 *               if (template == null) {
 *                   Set columns = new LinkedHashSet();                  // every value of those properties
 *                   for (Object p : rows) columns.addAll(((Property) p).getPossibleValues());
 *                   template = ArrayTable.create(rows, columns);
 *                   tables.putIfAbsent(rows, template);
 *               }
 *               ArrayTable table = ArrayTable.create(template);        // Guava's copy shares the index maps
 *               table.putAll(t);
 *               return table; }
 * </pre>
 * Every cell holds the same state as before, and get(property, value) answers the same for every key (null where
 * there is no neighbour), which is all the game does with the table (setValue, trySetValue). The table only has a
 * few more empty cells (a state's own values), which only code that walks the whole table could see; the patch is
 * applied only when nothing else in the class reads the field.
 *
 * FerriteCore replaces these tables with its own; with it the patch stays off.
 */
final class SharedStateTables extends Patch {
    static final String HELPER = Code.PREFIX + "table";
    static final String SHARE = Code.PREFIX + "share";
    static final String CACHE = Code.PREFIX + "tables";
    static final String TABLE = "com/google/common/collect/Table";
    static final String ARRAY_TABLE = "com/google/common/collect/ArrayTable";
    static final String CREATE = "(L" + TABLE + ";)L" + ARRAY_TABLE + ";";

    SharedStateTables() {
        super("shared-state-tables", 1, "block states share the index maps of their neighbour tables (about 13 MB "
                + "less memory in Minecraft 1.20 - 1.21.1)",
                new String[][] {{ARRAY_TABLE, "com/google/common/collect/HashBasedTable", CREATE}},
                new String[] {"ferritecore"},
                "net/minecraft/world/level/block/state/StateHolder", "net/minecraft/class_2688");
    }

    @Override
    void apply(Target t, Ctx ctx) throws Skip {
        List<Object[]> calls = t.calls(Opcodes.INVOKESTATIC, ARRAY_TABLE, "create", CREATE);
        if (calls.isEmpty()) {
            throw Skip.notTarget("no ArrayTable.create(Table)");
        }
        if (calls.size() != 1) {
            throw Skip.because("ArrayTable.create is called " + calls.size() + " times");
        }
        MethodNode populate = (MethodNode) calls.get(0)[0];
        AbstractInsnNode create = (AbstractInsnNode) calls.get(0)[1];
        AbstractInsnNode put = Code.next(create);
        AbstractInsnNode load = Code.prev(create);
        if (!(put instanceof FieldInsnNode) || put.getOpcode() != Opcodes.PUTFIELD || load == null
                || load.getOpcode() != Opcodes.ALOAD || !((FieldInsnNode) put).owner.equals(t.name)
                || !((FieldInsnNode) put).desc.equals("L" + TABLE + ";") || !populate.desc.equals("(Ljava/util/Map;)V")
                || (populate.access & Opcodes.ACC_STATIC) != 0) {
            throw Skip.notTarget("not the neighbour table");
        }
        String field = ((FieldInsnNode) put).name;

        // the property's getPossibleValues(): the only ()Collection call on a game class in populateNeighbours
        MethodInsnNode values = null;
        for (AbstractInsnNode n = populate.instructions.getFirst(); n != null; n = n.getNext()) {
            if (n instanceof MethodInsnNode && n.getOpcode() == Opcodes.INVOKEVIRTUAL
                    && ((MethodInsnNode) n).desc.equals("()Ljava/util/Collection;")
                    && !((MethodInsnNode) n).owner.startsWith("java/")) {
                if (values != null) {
                    throw Skip.because("more than one list of values in populateNeighbours");
                }
                values = (MethodInsnNode) n;
            }
        }
        if (values == null) {
            throw Skip.because("no property values in populateNeighbours");
        }

        // the table is only used through get(property, value), plus the null check and the store in populateNeighbours
        int access = t.fieldAccess(field, "L" + TABLE + ";");
        if (access < 0 || (access & Opcodes.ACC_PRIVATE) == 0 || (access & Opcodes.ACC_STATIC) != 0) {
            throw Skip.because("the neighbour table is not a private field");
        }
        int gets = 0;
        for (MethodNode m : t.fieldUsers(t.name, field, "L" + TABLE + ";")) {
            for (AbstractInsnNode n = m.instructions.getFirst(); n != null; n = n.getNext()) {
                if (!(n instanceof FieldInsnNode) || !((FieldInsnNode) n).owner.equals(t.name)
                        || !((FieldInsnNode) n).name.equals(field)) {
                    continue;
                }
                if (n == put) {
                    continue;
                }
                AbstractInsnNode a = Code.next(n);
                AbstractInsnNode[] get = Code.after(n, Opcodes.ALOAD, Opcodes.ALOAD, Opcodes.INVOKEINTERFACE);
                boolean nullCheck = m == populate && a != null
                        && (a.getOpcode() == Opcodes.IFNULL || a.getOpcode() == Opcodes.IFNONNULL);
                boolean lookup = get != null && Code.isCall(get[2], Opcodes.INVOKEINTERFACE, TABLE, "get",
                        "(Ljava/lang/Object;Ljava/lang/Object;)Ljava/lang/Object;");
                if (n.getOpcode() != Opcodes.GETFIELD || !(nullCheck || lookup)) {
                    throw Skip.because("the neighbour table is used in another way (" + m.name + m.desc + ")");
                }
                gets += lookup ? 1 : 0;
            }
        }
        if (gets == 0) {
            throw Skip.because("the neighbour table is never read");
        }
        if (Code.changedByMixin(populate)) {
            throw Skip.because("populateNeighbours is changed by a mod");
        }

        t.addField(new FieldNode(Opcodes.ACC_PRIVATE | Opcodes.ACC_STATIC | Opcodes.ACC_SYNTHETIC, CACHE,
                "Ljava/util/Map;", null, null));
        if (!t.needsFrames()) {
            throw Skip.because("class file too old");
        }
        MethodNode share = share(t.name, values);
        MethodNode helper = guard(t.name, share);
        t.add(share);
        t.add(helper);
        populate.instructions.set(create, new MethodInsnNode(Opcodes.INVOKESTATIC, t.name, helper.name, helper.desc,
                t.isInterface()));
        t.changed(populate);
    }

    /** The sharing above (blockforge$lite$share), with hand-written frames. */
    static MethodNode share(String owner, MethodInsnNode possibleValues) {
        MethodNode h = new MethodNode(Opcodes.ACC_PRIVATE | Opcodes.ACC_STATIC | Opcodes.ACC_SYNTHETIC, SHARE, CREATE,
                null, null);
        Label have = new Label();
        Label loop = new Label();
        Label made = new Label();
        Label copy = new Label();
        String list = "java/util/ArrayList";
        String set = "java/util/LinkedHashSet";
        h.visitCode();
        // 0 t, 1 rows, 2 cache, 3 template, 4 columns, 5 iterator
        h.visitTypeInsn(Opcodes.NEW, list);
        h.visitInsn(Opcodes.DUP);
        h.visitVarInsn(Opcodes.ALOAD, 0);
        h.visitMethodInsn(Opcodes.INVOKEINTERFACE, TABLE, "rowKeySet", "()Ljava/util/Set;", true);
        h.visitMethodInsn(Opcodes.INVOKESPECIAL, list, "<init>", "(Ljava/util/Collection;)V", false);
        h.visitVarInsn(Opcodes.ASTORE, 1);
        h.visitFieldInsn(Opcodes.GETSTATIC, owner, CACHE, "Ljava/util/Map;");
        h.visitVarInsn(Opcodes.ASTORE, 2);
        h.visitVarInsn(Opcodes.ALOAD, 2);
        h.visitJumpInsn(Opcodes.IFNONNULL, have);
        // a race here only makes a second cache; tables made from either one are correct
        h.visitTypeInsn(Opcodes.NEW, "java/util/concurrent/ConcurrentHashMap");
        h.visitInsn(Opcodes.DUP);
        h.visitMethodInsn(Opcodes.INVOKESPECIAL, "java/util/concurrent/ConcurrentHashMap", "<init>", "()V", false);
        h.visitInsn(Opcodes.DUP);
        h.visitFieldInsn(Opcodes.PUTSTATIC, owner, CACHE, "Ljava/util/Map;");
        h.visitVarInsn(Opcodes.ASTORE, 2);
        h.visitLabel(have);
        h.visitFrame(Opcodes.F_FULL, 3, new Object[] {TABLE, list, "java/util/Map"}, 0, new Object[0]);
        h.visitVarInsn(Opcodes.ALOAD, 2);
        h.visitVarInsn(Opcodes.ALOAD, 1);
        h.visitMethodInsn(Opcodes.INVOKEINTERFACE, "java/util/Map", "get", "(Ljava/lang/Object;)Ljava/lang/Object;",
                true);
        h.visitVarInsn(Opcodes.ASTORE, 3);
        h.visitVarInsn(Opcodes.ALOAD, 3);
        h.visitJumpInsn(Opcodes.IFNONNULL, copy);
        h.visitTypeInsn(Opcodes.NEW, set);
        h.visitInsn(Opcodes.DUP);
        h.visitMethodInsn(Opcodes.INVOKESPECIAL, set, "<init>", "()V", false);
        h.visitVarInsn(Opcodes.ASTORE, 4);
        h.visitVarInsn(Opcodes.ALOAD, 1);
        h.visitMethodInsn(Opcodes.INVOKEVIRTUAL, list, "iterator", "()Ljava/util/Iterator;", false);
        h.visitVarInsn(Opcodes.ASTORE, 5);
        h.visitLabel(loop);
        h.visitFrame(Opcodes.F_FULL, 6, new Object[] {TABLE, list, "java/util/Map", "java/lang/Object", set,
            "java/util/Iterator"}, 0, new Object[0]);
        h.visitVarInsn(Opcodes.ALOAD, 5);
        h.visitMethodInsn(Opcodes.INVOKEINTERFACE, "java/util/Iterator", "hasNext", "()Z", true);
        h.visitJumpInsn(Opcodes.IFEQ, made);
        h.visitVarInsn(Opcodes.ALOAD, 4);
        h.visitVarInsn(Opcodes.ALOAD, 5);
        h.visitMethodInsn(Opcodes.INVOKEINTERFACE, "java/util/Iterator", "next", "()Ljava/lang/Object;", true);
        h.visitTypeInsn(Opcodes.CHECKCAST, possibleValues.owner);
        h.visitMethodInsn(Opcodes.INVOKEVIRTUAL, possibleValues.owner, possibleValues.name, possibleValues.desc, false);
        h.visitMethodInsn(Opcodes.INVOKEVIRTUAL, set, "addAll", "(Ljava/util/Collection;)Z", false);
        h.visitInsn(Opcodes.POP);
        h.visitJumpInsn(Opcodes.GOTO, loop);
        h.visitLabel(made);
        h.visitFrame(Opcodes.F_SAME, 0, null, 0, null);
        h.visitVarInsn(Opcodes.ALOAD, 1);
        h.visitVarInsn(Opcodes.ALOAD, 4);
        h.visitMethodInsn(Opcodes.INVOKESTATIC, ARRAY_TABLE, "create",
                "(Ljava/lang/Iterable;Ljava/lang/Iterable;)L" + ARRAY_TABLE + ";", false);
        h.visitVarInsn(Opcodes.ASTORE, 3);
        h.visitVarInsn(Opcodes.ALOAD, 2);
        h.visitVarInsn(Opcodes.ALOAD, 1);
        h.visitVarInsn(Opcodes.ALOAD, 3);
        h.visitMethodInsn(Opcodes.INVOKEINTERFACE, "java/util/Map", "putIfAbsent",
                "(Ljava/lang/Object;Ljava/lang/Object;)Ljava/lang/Object;", true);
        h.visitInsn(Opcodes.POP);
        h.visitLabel(copy);
        h.visitFrame(Opcodes.F_FULL, 4, new Object[] {TABLE, list, "java/util/Map", "java/lang/Object"}, 0,
                new Object[0]);
        h.visitVarInsn(Opcodes.ALOAD, 3);
        h.visitTypeInsn(Opcodes.CHECKCAST, ARRAY_TABLE);
        h.visitMethodInsn(Opcodes.INVOKESTATIC, ARRAY_TABLE, "create", CREATE, false);
        h.visitInsn(Opcodes.DUP);
        h.visitVarInsn(Opcodes.ALOAD, 0);
        h.visitMethodInsn(Opcodes.INVOKEVIRTUAL, ARRAY_TABLE, "putAll", "(L" + TABLE + ";)V", false);
        h.visitInsn(Opcodes.ARETURN);
        h.visitMaxs(4, 6);
        h.visitEnd();
        return h;
    }

    /**
     * static ArrayTable blockforge$lite$table(Table t) { try { return share(t); } catch (RuntimeException e) { return
     * ArrayTable.create(t); } } - a mod's odd property (say, one whose equals() makes two rows the same) gets the
     * game's own table instead of an error.
     */
    static MethodNode guard(String owner, MethodNode share) {
        MethodNode h = new MethodNode(Opcodes.ACC_PRIVATE | Opcodes.ACC_STATIC | Opcodes.ACC_SYNTHETIC, HELPER, CREATE,
                null, null);
        Label start = new Label();
        Label end = new Label();
        Label handler = new Label();
        h.visitCode();
        h.visitTryCatchBlock(start, end, handler, "java/lang/RuntimeException");
        h.visitLabel(start);
        h.visitVarInsn(Opcodes.ALOAD, 0);
        h.visitMethodInsn(Opcodes.INVOKESTATIC, owner, share.name, share.desc, false);
        h.visitLabel(end);
        h.visitInsn(Opcodes.ARETURN);
        h.visitLabel(handler);
        h.visitFrame(Opcodes.F_SAME1, 0, null, 1, new Object[] {"java/lang/RuntimeException"});
        h.visitInsn(Opcodes.POP);
        h.visitVarInsn(Opcodes.ALOAD, 0);
        h.visitMethodInsn(Opcodes.INVOKESTATIC, ARRAY_TABLE, "create", CREATE, false);
        h.visitInsn(Opcodes.ARETURN);
        h.visitMaxs(1, 1);
        h.visitEnd();
        return h;
    }
}
