package net.minecraft.client.renderer;

import it.unimi.dsi.fastutil.objects.ObjectArrayList;
import net.minecraft.client.renderer.chunk.SectionRenderDispatcher;

/** Stand-in of the game's class (see {@link net.minecraft.client.Minecraft}). */
public class LevelRenderer {
    public ObjectArrayList<SectionRenderDispatcher.RenderSection> sections =
            new ObjectArrayList<SectionRenderDispatcher.RenderSection>();

    public ObjectArrayList<SectionRenderDispatcher.RenderSection> visibleSections() {
        return sections;
    }
}
