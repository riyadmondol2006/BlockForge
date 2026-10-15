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
import java.awt.Component;
import java.awt.FlowLayout;
import java.awt.GridBagConstraints;
import java.awt.GridBagLayout;
import java.util.List;

import javax.swing.BorderFactory;
import javax.swing.Box;
import javax.swing.BoxLayout;
import javax.swing.JLabel;
import javax.swing.JMenuItem;
import javax.swing.JPanel;
import javax.swing.JPopupMenu;
import javax.swing.SwingUtilities;

import org.mini2Dx.gettext.GetText;

import com.blockforge.launcher.data.MicrosoftAccount;
import com.blockforge.launcher.gui.dialogs.LoginWithMicrosoftDialog;
import com.blockforge.launcher.managers.AccountManager;
import com.blockforge.launcher.managers.DialogManager;

import io.reactivex.rxjava3.disposables.CompositeDisposable;

/**
 * Manage the players you can launch Minecraft as. Offline players need nothing, Microsoft sign in is optional.
 */
public class AccountsPage extends Page {
    private static final long serialVersionUID = 1L;

    private final JPanel list = new JPanel();
    private final SearchBox nameField = new SearchBox(GetText.tr("Player name"), false, 18);
    private final JLabel error = Lbl.text("", 12f, false, Palette::danger);
    private final CompositeDisposable subscriptions = new CompositeDisposable();

    public AccountsPage() {
        JPanel column = new JPanel();
        column.setOpaque(false);
        column.setLayout(new BoxLayout(column, BoxLayout.Y_AXIS));
        column.setBorder(BorderFactory.createEmptyBorder(28, 36, 36, 36));

        JLabel title = Lbl.title(GetText.tr("Accounts"));
        title.setAlignmentX(LEFT_ALIGNMENT);
        JLabel sub = Lbl.dim(GetText.tr("Play offline with any name. Signing in with Microsoft is optional."));
        sub.setAlignmentX(LEFT_ALIGNMENT);
        column.add(title);
        column.add(sub);
        column.add(Box.createVerticalStrut(24));

        list.setOpaque(false);
        list.setLayout(new BoxLayout(list, BoxLayout.Y_AXIS));
        list.setAlignmentX(LEFT_ALIGNMENT);
        column.add(list);
        column.add(Box.createVerticalStrut(20));
        column.add(addOfflineCard());
        column.add(Box.createVerticalStrut(14));
        column.add(microsoftCard());

        JPanel top = new JPanel(new BorderLayout());
        top.setOpaque(false);
        JPanel limited = new JPanel(new BorderLayout());
        limited.setOpaque(false);
        limited.add(column, BorderLayout.WEST);
        top.add(limited, BorderLayout.NORTH);
        add(FitWidthPanel.scroll(fit(top)), BorderLayout.CENTER);
    }

    private static JPanel fit(JPanel inner) {
        FitWidthPanel outer = new FitWidthPanel(0);
        outer.setLayout(new BorderLayout());
        outer.add(inner, BorderLayout.CENTER);
        return outer;
    }

    private RoundPanel addOfflineCard() {
        RoundPanel card = new RoundPanel(new BorderLayout(0, 12), 22, Palette::card);
        card.line(Palette::outline);
        card.pad(18, 20, 18, 20);
        card.setAlignmentX(LEFT_ALIGNMENT);
        card.setMaximumSize(new java.awt.Dimension(640, 170));

        JPanel text = new JPanel();
        text.setOpaque(false);
        text.setLayout(new BoxLayout(text, BoxLayout.Y_AXIS));
        JLabel heading = Lbl.heading(GetText.tr("Add an offline player"));
        JLabel hint = Lbl.dim(GetText.tr("3-16 letters, numbers or underscores. No account needed."));
        heading.setAlignmentX(LEFT_ALIGNMENT);
        hint.setAlignmentX(LEFT_ALIGNMENT);
        error.setAlignmentX(LEFT_ALIGNMENT);
        text.add(heading);
        text.add(hint);
        card.add(text, BorderLayout.NORTH);

        JPanel row = new JPanel(new BorderLayout(12, 0));
        row.setOpaque(false);
        row.add(nameField, BorderLayout.CENTER);
        Btn add = new Btn(GetText.tr("Add"), Btn.Kind.FILLED).icon(Icons.PLUS);
        row.add(add, BorderLayout.EAST);
        JPanel bottom = new JPanel(new BorderLayout(0, 4));
        bottom.setOpaque(false);
        bottom.add(row, BorderLayout.CENTER);
        bottom.add(error, BorderLayout.SOUTH);
        card.add(bottom, BorderLayout.CENTER);

        Runnable submit = this::addOffline;
        add.addActionListener(e -> submit.run());
        nameField.addActionListener(e -> submit.run());
        return card;
    }

    private void addOffline() {
        String name = nameField.getText().trim();
        if (!AccountManager.isValidOfflineUsername(name)) {
            error.setText(GetText.tr("Use 3-16 letters, numbers or underscores."));
            return;
        }
        if (AccountManager.addOfflineAccount(name) == null) {
            error.setText(GetText.tr("You already have an account with that name."));
            return;
        }
        error.setText(" ");
        nameField.setText("");
    }

    private RoundPanel microsoftCard() {
        RoundPanel card = new RoundPanel(new BorderLayout(16, 0), 22, Palette::card);
        card.line(Palette::outline);
        card.pad(18, 20, 18, 20);
        card.setAlignmentX(LEFT_ALIGNMENT);
        card.setMaximumSize(new java.awt.Dimension(640, 110));

        JPanel text = new JPanel();
        text.setOpaque(false);
        text.setLayout(new BoxLayout(text, BoxLayout.Y_AXIS));
        JLabel heading = Lbl.heading(GetText.tr("Microsoft account"));
        JLabel hint = Lbl.dim(GetText.tr("Optional. Gives you your real skin and lets you join online servers."));
        heading.setAlignmentX(LEFT_ALIGNMENT);
        hint.setAlignmentX(LEFT_ALIGNMENT);
        text.add(heading);
        text.add(hint);
        card.add(text, BorderLayout.CENTER);

        Btn signIn = new Btn(GetText.tr("Sign in"), Btn.Kind.TONAL).icon(Icons.PERSON);
        signIn.addActionListener(e -> {
            int before = AccountManager.getAccounts().size();
            LoginWithMicrosoftDialog dialog = new LoginWithMicrosoftDialog();
            dialog.setVisible(true);
            if (before != AccountManager.getAccounts().size() && dialog.account != null) {
                dialog.account.updateSkin();
                rebuild();
            }
        });
        JPanel right = new JPanel(new GridBagLayout());
        right.setOpaque(false);
        right.add(signIn, new GridBagConstraints());
        card.add(right, BorderLayout.EAST);
        return card;
    }

    @Override
    public void addNotify() {
        super.addNotify();
        subscriptions.add(AccountManager.getAccountsObservable()
            .subscribe(accounts -> SwingUtilities.invokeLater(this::rebuild)));
        subscriptions.add(AccountManager.getSelectedAccountObservable()
            .subscribe(selected -> SwingUtilities.invokeLater(this::rebuild)));
    }

    @Override
    public void removeNotify() {
        subscriptions.clear();
        super.removeNotify();
    }

    private void rebuild() {
        List<MicrosoftAccount> accounts = AccountManager.getAccounts();
        MicrosoftAccount active = AccountManager.getSelectedAccount();

        list.removeAll();
        for (MicrosoftAccount account : accounts) {
            RoundPanel row = row(account, account == active);
            list.add(row);
            list.add(Box.createVerticalStrut(10));
        }
        list.revalidate();
        list.repaint();
    }

    private RoundPanel row(MicrosoftAccount account, boolean active) {
        RoundPanel row = new RoundPanel(new BorderLayout(16, 0), 22, active ? Palette::accentSoft : Palette::card);
        row.line(active ? Palette::accent : Palette::outline);
        row.hover(active ? Palette::accentSoft : Palette::cardHover);
        row.pad(14, 16, 14, 14);
        row.setAlignmentX(LEFT_ALIGNMENT);
        row.setMaximumSize(new java.awt.Dimension(640, 84));

        Avatar avatar = new Avatar(52);
        avatar.setAccount(account);
        JPanel avatarWrap = new JPanel(new GridBagLayout());
        avatarWrap.setOpaque(false);
        avatarWrap.add(avatar);
        row.add(avatarWrap, BorderLayout.WEST);

        JPanel text = new JPanel();
        text.setOpaque(false);
        text.setLayout(new BoxLayout(text, BoxLayout.Y_AXIS));
        JLabel name = Lbl.text(account.minecraftUsername, 16f, true, Palette::text);
        JLabel kind = Lbl.dim(account.offline ? GetText.tr("Offline player") : GetText.tr("Microsoft account"));
        name.setAlignmentX(LEFT_ALIGNMENT);
        kind.setAlignmentX(LEFT_ALIGNMENT);
        text.add(Box.createVerticalGlue());
        text.add(name);
        text.add(kind);
        text.add(Box.createVerticalGlue());
        row.add(text, BorderLayout.CENTER);

        JPanel actions = new JPanel(new FlowLayout(FlowLayout.RIGHT, 8, 0));
        actions.setOpaque(false);
        Btn skin = new Btn(GetText.tr("Change skin"), Btn.Kind.TONAL).icon(Icons.EDIT).compact();
        skin.addActionListener(e -> SkinDialog.open(account));
        actions.add(skin);
        if (active) {
            JLabel badge = Lbl.text(GetText.tr("In use"), 13f, true, Palette::accent);
            badge.setIcon(Icons.CHECK.asIcon(18, Palette.accent()));
            badge.setIconTextGap(6);
            actions.add(badge);
        } else {
            Btn use = new Btn(GetText.tr("Use"), Btn.Kind.TONAL).compact();
            use.addActionListener(e -> AccountManager.switchAccount(account));
            actions.add(use);
        }
        Btn more = Btn.iconButton(Icons.MORE, GetText.tr("More options"));
        more.addActionListener(e -> menu(account).show(more, 0, more.getHeight()));
        actions.add(more);
        JPanel actionsWrap = new JPanel(new GridBagLayout());
        actionsWrap.setOpaque(false);
        actionsWrap.add(actions);
        row.add(actionsWrap, BorderLayout.EAST);
        return row;
    }

    private JPopupMenu menu(MicrosoftAccount account) {
        JPopupMenu menu = new JPopupMenu();
        if (!account.offline) {
            JMenuItem skin = new JMenuItem(GetText.tr("Change skin"));
            skin.addActionListener(e -> SkinDialog.open(account));
            menu.add(skin);
            JMenuItem reload = new JMenuItem(GetText.tr("Reload skin"));
            reload.addActionListener(e -> new Thread(() -> {
                // downloads the skin, so keep it off the UI thread
                account.updateSkin();
                SwingUtilities.invokeLater(this::rebuild);
            }, "BlockForge skin reload").start());
            menu.add(reload);
            JMenuItem username = new JMenuItem(GetText.tr("Update username"));
            username.addActionListener(e -> {
                account.updateUsername();
                rebuild();
            });
            menu.add(username);
            menu.addSeparator();
        }
        JMenuItem remove = new JMenuItem(GetText.tr("Remove"));
        remove.setForeground(Palette.danger());
        remove.addActionListener(e -> {
            int ret = DialogManager.yesNoDialog().setTitle(GetText.tr("Remove account"))
                .setContent(GetText.tr("Remove {0} from the launcher?", account.minecraftUsername))
                .setType(DialogManager.WARNING).show();
            if (ret == DialogManager.YES_OPTION) {
                AccountManager.removeAccount(account);
            }
        });
        menu.add(remove);
        return menu;
    }

    @Override
    public void onShow() {
        rebuild();
    }

}
