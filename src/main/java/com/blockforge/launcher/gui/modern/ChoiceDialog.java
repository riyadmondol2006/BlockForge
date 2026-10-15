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

import java.awt.BorderLayout;
import java.awt.Color;
import java.awt.Dialog;
import java.awt.Dimension;
import java.awt.FlowLayout;
import java.awt.Window;

import javax.swing.BorderFactory;
import javax.swing.JComponent;
import javax.swing.JDialog;
import javax.swing.JLabel;
import javax.swing.JPanel;
import javax.swing.KeyStroke;
import javax.swing.WindowConstants;

import com.blockforge.launcher.constants.Constants;

/**
 * A small question in the BlockForge style: an icon, a title, a few lines of text and a row of buttons. The last
 * button is the suggested one.
 */
public final class ChoiceDialog extends JDialog {
    private static final long serialVersionUID = 1L;

    private int choice = -1;

    private ChoiceDialog(Window owner, Icons icon, String title, String text, String... options) {
        super(owner, Constants.LAUNCHER_NAME, Dialog.ModalityType.DOCUMENT_MODAL);
        setDefaultCloseOperation(WindowConstants.DISPOSE_ON_CLOSE);

        JPanel root = new JPanel(new BorderLayout(0, 20)) {
            private static final long serialVersionUID = 1L;

            @Override
            public Color getBackground() {
                return Palette.bg();
            }
        };
        root.setBorder(BorderFactory.createEmptyBorder(26, 28, 22, 28));

        JPanel head = new JPanel(new BorderLayout(14, 0));
        head.setOpaque(false);
        RoundPanel badge = new RoundPanel(new BorderLayout(), 16, Palette::accentSoft);
        badge.setPreferredSize(new Dimension(44, 44));
        badge.add(new JLabel(icon.asIcon(24, Palette::accent)), BorderLayout.CENTER);
        JPanel badgeWrap = new JPanel(new BorderLayout());
        badgeWrap.setOpaque(false);
        badgeWrap.add(badge, BorderLayout.NORTH);
        head.add(badgeWrap, BorderLayout.WEST);

        JPanel words = new JPanel(new BorderLayout(0, 8));
        words.setOpaque(false);
        words.add(Lbl.text(title, 18f, true, Palette::text), BorderLayout.NORTH);
        words.add(new Lbl.Paragraph(text, 13.5f, 12, 400, Palette::textDim), BorderLayout.CENTER);
        head.add(words, BorderLayout.CENTER);
        root.add(head, BorderLayout.CENTER);

        JPanel buttons = new JPanel(new FlowLayout(FlowLayout.RIGHT, 10, 0));
        buttons.setOpaque(false);
        for (int i = 0; i < options.length; i++) {
            final int index = i;
            boolean primary = i == options.length - 1;
            Btn b = new Btn(options[i], primary ? Btn.Kind.FILLED : Btn.Kind.TONAL);
            b.addActionListener(e -> {
                choice = index;
                dispose();
            });
            buttons.add(b);
            if (primary) {
                getRootPane().setDefaultButton(b);
            }
        }
        root.add(buttons, BorderLayout.SOUTH);

        getRootPane().registerKeyboardAction(e -> dispose(), KeyStroke.getKeyStroke("ESCAPE"),
            JComponent.WHEN_IN_FOCUSED_WINDOW);
        setContentPane(root);
        pack();
        setMinimumSize(new Dimension(500, getHeight()));
        setLocationRelativeTo(owner);
    }

    /**
     * Asks a question and waits for the answer.
     *
     * @return the index of the button pressed, or -1 if the window was closed
     */
    public static int ask(Window owner, Icons icon, String title, String text, String... options) {
        ChoiceDialog dialog = new ChoiceDialog(owner, icon, title, text, options);
        dialog.setVisible(true);
        return dialog.choice;
    }
}
