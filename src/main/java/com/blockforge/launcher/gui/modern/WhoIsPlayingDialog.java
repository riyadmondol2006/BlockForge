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
import java.awt.CardLayout;
import java.awt.Component;
import java.awt.Cursor;
import java.awt.Dialog;
import java.awt.Dimension;
import java.awt.Frame;
import java.awt.GridBagLayout;
import java.awt.GridLayout;
import java.awt.event.MouseAdapter;
import java.awt.event.MouseEvent;
import java.util.List;

import javax.swing.BorderFactory;
import javax.swing.Box;
import javax.swing.BoxLayout;
import javax.swing.JDialog;
import javax.swing.JLabel;
import javax.swing.JPanel;
import javax.swing.WindowConstants;

import org.mini2Dx.gettext.GetText;

import com.blockforge.launcher.App;
import com.blockforge.launcher.constants.Constants;
import com.blockforge.launcher.data.MicrosoftAccount;
import com.blockforge.launcher.gui.dialogs.LoginWithMicrosoftDialog;
import com.blockforge.launcher.managers.AccountManager;
import com.blockforge.launcher.utils.Utils;

/**
 * Shown when BlockForge opens. The first question is simple: sign in with Microsoft, or play offline. Players that
 * were used before are listed so they are one click away.
 */
public final class WhoIsPlayingDialog extends JDialog {
    private static final long serialVersionUID = 1L;

    private final CardLayout steps = new CardLayout();
    private final JPanel stepPanel = new JPanel(steps);
    private final SearchBox nameField = new SearchBox(GetText.tr("Player name"), false, 18);
    private final JLabel error = Lbl.text(" ", 12f, false, Palette::danger);

    private WhoIsPlayingDialog(Frame owner) {
        super(owner, Constants.LAUNCHER_NAME, Dialog.ModalityType.APPLICATION_MODAL);
        setDefaultCloseOperation(WindowConstants.DISPOSE_ON_CLOSE);
        setResizable(false);
        setIconImage(Utils.getImage("/assets/image/icon.png"));

        JPanel root = new JPanel(new BorderLayout()) {
            private static final long serialVersionUID = 1L;

            @Override
            public java.awt.Color getBackground() {
                return Palette.bg();
            }
        };
        root.setBorder(BorderFactory.createEmptyBorder(30, 36, 30, 36));

        stepPanel.setOpaque(false);
        stepPanel.add(chooseStep(), "choose");
        stepPanel.add(nameStep(), "name");
        root.add(stepPanel, BorderLayout.CENTER);
        add(root);

        int saved = Math.min(AccountManager.getAccounts().size(), 3);
        setPreferredSize(new Dimension(560, 330 + (saved == 0 ? 0 : 36 + saved * 66)));
        pack();
        setLocationRelativeTo(owner);
    }

    private JPanel chooseStep() {
        JPanel column = new JPanel();
        column.setOpaque(false);
        column.setLayout(new BoxLayout(column, BoxLayout.Y_AXIS));

        JLabel title = Lbl.title(GetText.tr("Who's playing?"));
        JLabel sub = Lbl.dim(GetText.tr("Sign in with your Microsoft account, or play offline with any name."));
        title.setAlignmentX(Component.LEFT_ALIGNMENT);
        sub.setAlignmentX(Component.LEFT_ALIGNMENT);
        column.add(title);
        column.add(sub);
        column.add(Box.createVerticalStrut(22));

        JPanel choices = new JPanel(new GridLayout(1, 2, 14, 0));
        choices.setOpaque(false);
        choices.setAlignmentX(Component.LEFT_ALIGNMENT);
        choices.setMaximumSize(new Dimension(Integer.MAX_VALUE, 150));
        choices.add(choice(Icons.PERSON, GetText.tr("Sign in"), GetText.tr("Use your Microsoft account"),
            this::signIn));
        choices.add(choice(Icons.PLAY, GetText.tr("Play offline"), GetText.tr("Pick any player name"),
            () -> {
                steps.show(stepPanel, "name");
                nameField.requestFocusInWindow();
            }));
        column.add(choices);

        List<MicrosoftAccount> accounts = AccountManager.getAccounts();
        if (!accounts.isEmpty()) {
            column.add(Box.createVerticalStrut(22));
            JLabel saved = Lbl.text(GetText.tr("CONTINUE AS"), 11f, true, Palette::textDim);
            saved.setAlignmentX(Component.LEFT_ALIGNMENT);
            column.add(saved);
            column.add(Box.createVerticalStrut(8));
            for (int i = 0; i < Math.min(accounts.size(), 3); i++) {
                column.add(playerRow(accounts.get(i)));
                column.add(Box.createVerticalStrut(8));
            }
        }
        return column;
    }

    private RoundPanel choice(Icons icon, String title, String subtitle, Runnable action) {
        RoundPanel card = new RoundPanel(new BorderLayout(0, 10), 24, Palette::card);
        card.line(Palette::outline);
        card.hover(Palette::cardHover);
        card.pad(18, 18, 18, 18);
        card.setCursor(Cursor.getPredefinedCursor(Cursor.HAND_CURSOR));

        JLabel iconLabel = new JLabel(icon.asIcon(30, Palette::accent));
        iconLabel.setHorizontalAlignment(JLabel.LEFT);
        card.add(iconLabel, BorderLayout.NORTH);
        JPanel text = new JPanel();
        text.setOpaque(false);
        text.setLayout(new BoxLayout(text, BoxLayout.Y_AXIS));
        JLabel t = Lbl.text(title, 16f, true, Palette::text);
        JLabel s = Lbl.text(subtitle, 12f, false, Palette::textDim);
        text.add(t);
        text.add(s);
        card.add(text, BorderLayout.CENTER);

        MouseAdapter click = new MouseAdapter() {
            @Override
            public void mouseClicked(MouseEvent e) {
                action.run();
            }
        };
        card.addMouseListener(click);
        iconLabel.addMouseListener(click);
        text.addMouseListener(click);
        t.addMouseListener(click);
        s.addMouseListener(click);
        return card;
    }

    private JPanel nameStep() {
        JPanel column = new JPanel();
        column.setOpaque(false);
        column.setLayout(new BoxLayout(column, BoxLayout.Y_AXIS));

        JLabel title = Lbl.title(GetText.tr("Play offline"));
        JLabel sub = Lbl.dim(GetText.tr("3-16 letters, numbers or underscores. No account needed."));
        title.setAlignmentX(Component.LEFT_ALIGNMENT);
        sub.setAlignmentX(Component.LEFT_ALIGNMENT);
        column.add(title);
        column.add(sub);
        column.add(Box.createVerticalStrut(22));

        nameField.setAlignmentX(Component.LEFT_ALIGNMENT);
        nameField.setMaximumSize(new Dimension(Integer.MAX_VALUE, 44));
        nameField.addActionListener(e -> useName());
        column.add(nameField);
        error.setAlignmentX(Component.LEFT_ALIGNMENT);
        error.setBorder(BorderFactory.createEmptyBorder(6, 6, 0, 0));
        column.add(error);
        column.add(Box.createVerticalStrut(18));

        JPanel buttons = new JPanel(new java.awt.FlowLayout(java.awt.FlowLayout.LEFT, 10, 0));
        buttons.setOpaque(false);
        buttons.setAlignmentX(Component.LEFT_ALIGNMENT);
        Btn play = new Btn(GetText.tr("Continue"), Btn.Kind.FILLED).icon(Icons.PLAY);
        play.addActionListener(e -> useName());
        Btn back = new Btn(GetText.tr("Back"), Btn.Kind.TONAL);
        back.addActionListener(e -> steps.show(stepPanel, "choose"));
        buttons.add(play);
        buttons.add(back);
        column.add(buttons);
        return column;
    }

    private JPanel playerRow(MicrosoftAccount account) {
        RoundPanel row = new RoundPanel(new BorderLayout(14, 0), 20, Palette::card);
        row.line(Palette::outline);
        row.hover(Palette::cardHover);
        row.pad(8, 12, 8, 12);
        row.setAlignmentX(Component.LEFT_ALIGNMENT);
        row.setMaximumSize(new Dimension(Integer.MAX_VALUE, 58));
        row.setCursor(Cursor.getPredefinedCursor(Cursor.HAND_CURSOR));

        Avatar avatar = new Avatar(38);
        avatar.setAccount(account);
        JPanel avatarWrap = new JPanel(new GridBagLayout());
        avatarWrap.setOpaque(false);
        avatarWrap.add(avatar);
        row.add(avatarWrap, BorderLayout.WEST);

        JPanel text = new JPanel();
        text.setOpaque(false);
        text.setLayout(new BoxLayout(text, BoxLayout.Y_AXIS));
        JLabel name = Lbl.text(account.minecraftUsername, 14f, true, Palette::text);
        JLabel kind = Lbl.text(account.offline ? GetText.tr("Offline player") : GetText.tr("Microsoft account"),
            12f, false, Palette::textDim);
        text.add(Box.createVerticalGlue());
        text.add(name);
        text.add(kind);
        text.add(Box.createVerticalGlue());
        row.add(text, BorderLayout.CENTER);

        MouseAdapter pick = new MouseAdapter() {
            @Override
            public void mouseClicked(MouseEvent e) {
                AccountManager.switchAccount(account);
                dispose();
            }
        };
        row.addMouseListener(pick);
        avatarWrap.addMouseListener(pick);
        text.addMouseListener(pick);
        name.addMouseListener(pick);
        kind.addMouseListener(pick);
        return row;
    }

    private void useName() {
        String name = nameField.getText().trim();
        if (!AccountManager.isValidOfflineUsername(name)) {
            error.setText(GetText.tr("Use 3-16 letters, numbers or underscores."));
            return;
        }

        MicrosoftAccount existing = AccountManager.getAccountByName(name);
        if (existing != null) {
            AccountManager.switchAccount(existing);
        } else {
            AccountManager.addOfflineAccount(name);
        }
        dispose();
    }

    private void signIn() {
        int before = AccountManager.getAccounts().size();
        LoginWithMicrosoftDialog dialog = new LoginWithMicrosoftDialog();
        dialog.setVisible(true);
        if (before != AccountManager.getAccounts().size()) {
            if (dialog.account != null) {
                dialog.account.updateSkin();
            }
            dispose();
        }
    }

    /**
     * Opens the dialog when the user asked to be asked on start, or when there is no player to play as yet.
     */
    public static void askIfNeeded(Frame owner) {
        if (!App.settings.askPlayerOnStartup && AccountManager.getSelectedAccount() != null) {
            return;
        }
        new WhoIsPlayingDialog(owner).setVisible(true);
    }
}
