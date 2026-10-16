package net.minecraft.client.renderer.chunk;

/** Stand-in of the game's class (see {@link net.minecraft.client.Minecraft}). */
public class SectionRenderDispatcher {
    public static class RenderSection {
        public final long node;

        public RenderSection(long node) {
            this.node = node;
        }

        public long getSectionNode() {
            return node;
        }
    }
}
