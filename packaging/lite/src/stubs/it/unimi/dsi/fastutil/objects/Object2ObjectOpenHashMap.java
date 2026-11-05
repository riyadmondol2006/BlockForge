package it.unimi.dsi.fastutil.objects;

/** Stand-in of the library class the game uses; for the self-test it only has to be a Map made from a size. */
public class Object2ObjectOpenHashMap<K, V> extends java.util.HashMap<K, V> {
    private static final long serialVersionUID = 1L;

    public Object2ObjectOpenHashMap(int expected) {
        super(expected);
    }
}
