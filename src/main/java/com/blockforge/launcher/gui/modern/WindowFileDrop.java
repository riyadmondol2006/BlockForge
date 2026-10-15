/*
 * BlockForge - a fork of ATLauncher (https://github.com/ATLauncher/ATLauncher)
 *
 * This program is free software: you can redistribute it and/or modify
 * it under the terms of the GNU General Public License as published by
 * the Free Software Foundation, either version 3 of the License, or
 * (at your option) any later version.
 *
 * This program is distributed in the hope that it will be useful,
 * but WITHOUT ANY WARRANTY; without even the implied warranty of
 * MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE. See the
 * GNU General Public License for more details.
 *
 * You should have received a copy of the GNU General Public License
 * along with this program. If not, see <http://www.gnu.org/licenses/>.
 */
package com.blockforge.launcher.gui.modern;

import java.awt.Component;
import java.awt.Container;
import java.awt.Point;
import java.awt.Rectangle;
import java.awt.datatransfer.DataFlavor;
import java.awt.dnd.DnDConstants;
import java.awt.dnd.DropTarget;
import java.awt.dnd.DropTargetAdapter;
import java.awt.dnd.DropTargetDragEvent;
import java.awt.dnd.DropTargetDropEvent;
import java.awt.dnd.DropTargetEvent;
import java.io.File;
import java.util.ArrayList;
import java.util.List;
import java.util.function.Consumer;

import javax.swing.JComponent;
import javax.swing.RootPaneContainer;
import javax.swing.SwingUtilities;

import org.mini2Dx.gettext.GetText;

import com.blockforge.launcher.data.Instance;
import com.blockforge.launcher.managers.LogManager;

/**
 * Lets files be dropped anywhere on a window. While they are dragged over it an overlay says what will happen; a
 * drop on an instance card adds them to that instance, anywhere else asks which instance (or, in a window that
 * belongs to one instance, goes straight into it).
 */
public final class WindowFileDrop {
    private final java.awt.Window window;
    private final RootPaneContainer container;
    private final FileDropOverlay overlay;
    /** the instance this window is about, or null for the main window */
    private final Instance fixed;
    private final Consumer<Instance> changed;

    private WindowFileDrop(java.awt.Window window, Instance fixed, Consumer<Instance> changed) {
        this.window = window;
        this.container = (RootPaneContainer) window;
        this.fixed = fixed;
        this.changed = changed;
        this.overlay = fixed == null
            ? new FileDropOverlay(GetText.tr("Drop to add to an instance"),
                GetText.tr("Mods, resource packs, shaders and data packs. Modpacks become new instances."))
            : new FileDropOverlay(GetText.tr("Drop to add to {0}", fixed.launcher.name),
                GetText.tr("Mods go to mods, resource packs to resourcepacks, shaders to shaderpacks"));
    }

    /**
     * Turns on dropping files for a window.
     *
     * @param fixed   the instance everything goes into, or null to go by where the files are dropped
     * @param changed runs with the instance after files went into it
     */
    public static WindowFileDrop install(java.awt.Window window, Instance fixed, Consumer<Instance> changed) {
        WindowFileDrop drop = new WindowFileDrop(window, fixed, changed);
        drop.container.setGlassPane(drop.overlay);
        // the window takes the first drag events; once the overlay shows it covers everything and takes over
        new DropTarget(drop.container.getRootPane(), DnDConstants.ACTION_COPY, drop.new Listener(false), true);
        new DropTarget(drop.overlay, DnDConstants.ACTION_COPY, drop.new Listener(true), true);
        return drop;
    }

    public FileDropOverlay overlay() {
        return overlay;
    }

    /** Shows the overlay as if files were being dragged at a point of the window (null for no point). */
    public void preview(Point point) {
        overlay.setVisible(true);
        track(point);
    }

    public void hide() {
        overlay.setTarget(null, null);
        overlay.setVisible(false);
    }

    /** The instance under a point of the overlay, if any. */
    Instance instanceAt(Point point) {
        if (fixed != null) {
            return fixed;
        }
        Component deepest = componentAt(point);
        return FileDropFlow.instanceAt(deepest);
    }

    private Component componentAt(Point point) {
        if (point == null) {
            return null;
        }
        Container content = container.getContentPane();
        Point p = SwingUtilities.convertPoint(overlay, point, content);
        return SwingUtilities.getDeepestComponentAt(content, p.x, p.y);
    }

    private void track(Point point) {
        if (fixed != null) {
            return;
        }
        JComponent card = FileDropFlow.instanceComponentAt(componentAt(point));
        if (card == null || !card.isShowing()) {
            overlay.setTarget(null, null);
            return;
        }
        Rectangle bounds = SwingUtilities.convertRectangle(card.getParent(), card.getBounds(), overlay);
        Instance instance = FileDropFlow.instanceAt(card);
        overlay.setTarget(bounds, GetText.tr("Add to {0}", instance.launcher.name));
    }

    /** What a drop does: the same as the real drop handler, for the files given. */
    public void drop(List<File> files, Point point) {
        Instance target = instanceAt(point);
        hide();
        FileDropFlow.handle(window, files, target, changed);
    }

    private final class Listener extends DropTargetAdapter {
        private final boolean onOverlay;

        Listener(boolean onOverlay) {
            this.onOverlay = onOverlay;
        }

        @Override
        public void dragEnter(DropTargetDragEvent e) {
            if (!e.isDataFlavorSupported(DataFlavor.javaFileListFlavor)) {
                e.rejectDrag();
                return;
            }
            e.acceptDrag(DnDConstants.ACTION_COPY);
            overlay.setVisible(true);
            if (onOverlay) {
                track(e.getLocation());
            }
        }

        @Override
        public void dragOver(DropTargetDragEvent e) {
            if (onOverlay) {
                track(e.getLocation());
            }
        }

        @Override
        public void dragExit(DropTargetEvent e) {
            // leaving the window itself (not just handing over from the window to the overlay)
            if (onOverlay) {
                hide();
            }
        }

        @Override
        public void drop(DropTargetDropEvent e) {
            try {
                e.acceptDrop(DnDConstants.ACTION_COPY);
                @SuppressWarnings("unchecked")
                List<File> files = (List<File>) e.getTransferable().getTransferData(DataFlavor.javaFileListFlavor);
                e.dropComplete(true);
                Point point = onOverlay ? e.getLocation()
                    : SwingUtilities.convertPoint(container.getRootPane(), e.getLocation(), overlay);
                WindowFileDrop.this.drop(new ArrayList<>(files), point);
            } catch (Exception ex) {
                LogManager.logStackTrace("Couldn't take the dropped files", ex);
                e.dropComplete(false);
                hide();
            }
        }
    }
}
