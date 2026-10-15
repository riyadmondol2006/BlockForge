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

import java.awt.event.MouseAdapter;
import java.awt.event.MouseEvent;
import java.util.Optional;

import javax.swing.Box;
import javax.swing.JComponent;
import javax.swing.JPanel;
import javax.swing.SwingUtilities;

import org.mini2Dx.gettext.GetText;

import com.blockforge.launcher.App;
import com.blockforge.launcher.data.Instance;
import com.blockforge.launcher.data.QuickPlayOption;
import com.blockforge.launcher.mclauncher.GameFileWarmer;
import com.blockforge.launcher.utils.Worlds;

/**
 * The "Continue: world" button on Home: starts the game straight into the world played last, using Minecraft's Quick
 * Play (1.20 and newer). Older versions can't do that, so there is no button for them. Play itself is unchanged.
 */
final class ContinueWorld {
    private static final int MAX_NAME = 22;

    private ContinueWorld() {
    }

    /**
     * Adds the button after Play once the latest world has been found (off the Swing thread), and reads the game's
     * files ahead while the pointer is over the card.
     *
     * @param card     the card the buttons are on
     * @param buttons  the row of buttons, with Play first
     * @param instance the game on the card
     */
    static void attach(JComponent card, JPanel buttons, Instance instance) {
        card.addMouseListener(new MouseAdapter() {
            @Override
            public void mouseEntered(MouseEvent e) {
                GameFileWarmer.warmSoon(instance);
            }
        });

        if (!instance.isQuickPlaySupported(QuickPlayOption.singlePlayer)) {
            return;
        }

        App.TASKPOOL.execute(() -> {
            Optional<Worlds.World> world = Worlds.latest(instance.getRoot().resolve("saves"));
            world.ifPresent(w -> SwingUtilities.invokeLater(() -> add(buttons, instance, w)));
        });
    }

    private static void add(JPanel buttons, Instance instance, Worlds.World world) {
        String name = world.name.length() > MAX_NAME ? world.name.substring(0, MAX_NAME - 1) + "…" : world.name;
        Btn button = new Btn(GetText.tr("Continue: {0}", name), Btn.Kind.TONAL).icon(Icons.PLAY);
        button.setToolTipText(GetText.tr("Start straight into \"{0}\", the world you played last", world.name));
        button.addActionListener(e -> InstanceActions.continueWorld(instance, world.folder));

        int afterPlay = Math.min(1, buttons.getComponentCount());
        buttons.add(Box.createHorizontalStrut(10), afterPlay);
        buttons.add(button, afterPlay + 1);
        buttons.revalidate();
        buttons.repaint();
    }
}
