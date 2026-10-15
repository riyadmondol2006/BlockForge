/*
 * ATLauncher - https://github.com/ATLauncher/ATLauncher
 * Copyright (C) 2013-2022 ATLauncher
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
package com.blockforge.launcher.gui.dialogs.instancesettings;

import java.util.Arrays;
import java.util.List;

import javax.swing.JComboBox;
import javax.swing.JPanel;
import javax.swing.JTextField;

import org.mini2Dx.gettext.GetText;

import com.blockforge.launcher.builders.HTMLBuilder;
import com.blockforge.launcher.gui.modern.ModernDialogs;
import com.blockforge.launcher.data.Instance;
import com.blockforge.launcher.data.QuickPlayOption;
import com.blockforge.launcher.data.json.QuickPlay;
import com.blockforge.launcher.managers.AccountManager;
import com.blockforge.launcher.managers.DialogManager;
import com.blockforge.launcher.utils.ComboItem;
import com.blockforge.launcher.utils.ValidationUtils;

public class GeneralInstanceSettingsTab extends JPanel {
    private final Instance instance;

    private JComboBox<ComboItem<String>> account;

    private JComboBox<ComboItem<QuickPlayOption>> quickPlayType;

    private JPanel quickPlayServerAddressRow;
    private JTextField quickPlayServerAddress;

    private JPanel quickPlaySinglePlayerWorldRow;
    private JComboBox<ComboItem<String>> quickPlaySinglePlayerWorld;

    private JPanel quickPlayRealmIdRow;
    private JTextField quickPlayRealmId;

    public GeneralInstanceSettingsTab(Instance instance) {
        this.instance = instance;

        setupComponents();
    }

    private void setupComponents() {
        SettingRows.column(this);

        // Account

        account = new JComboBox<>();
        account.addItem(new ComboItem<>(null, GetText.tr("Use launcher default")));
        AccountManager.getAccounts().stream()
                .forEach(a -> account.addItem(new ComboItem<>(a.username, a.minecraftUsername)));

        for (int i = 0; i < account.getItemCount(); i++) {
            ComboItem<String> item = account.getItemAt(i);

            if ((item.getValue() == null && instance.launcher.account == null)
                    || (item.getValue() != null && item.getValue().equalsIgnoreCase(instance.launcher.account))) {
                account.setSelectedIndex(i);
                break;
            }
        }

        SettingRows.addSection(this, SettingRows.section(GetText.tr("Account"),
                SettingRows.row(GetText.tr("Play as"),
                        GetText.tr("The account used to start this instance. The launcher default is whichever account is selected in the launcher"),
                        SettingRows.dropdown(account))));

        // Quick play section

        QuickPlay quickPlay = instance.launcher.quickPlay;

        quickPlayType = new JComboBox<>();
        Arrays.stream(QuickPlayOption.compatibleValues(instance))
                .forEach(option -> quickPlayType.addItem(new ComboItem<>(option, option.label)));
        quickPlayType.setSelectedIndex(
                Arrays.asList(QuickPlayOption.compatibleValues(instance))
                        .indexOf(quickPlay.getSelectedQuickPlayOption()));

        // Code that is responsible for changing the input
        quickPlayType.addActionListener(e -> showInputForTheSelectedQuickPlayOption());

        // Quick play server address

        quickPlayServerAddress = new JTextField(13);
        quickPlayServerAddress.putClientProperty("JTextField.showClearButton", true);
        quickPlayServerAddress.putClientProperty("JTextField.clearCallback",
                (Runnable) () -> quickPlayServerAddress.setText(""));
        quickPlayServerAddress.setText(quickPlay.serverAddress);
        ModernDialogs.preferWidth(quickPlayServerAddress, SettingRows.CONTROL_WIDTH);
        quickPlayServerAddressRow = SettingRows.row(GetText.tr("Server address"),
                GetText.tr("The multiplayer server to join once the game has started"), quickPlayServerAddress);

        // Quick play select single player world dropdown

        quickPlaySinglePlayerWorld = new JComboBox<>();
        List<String> worldNames = instance.getSinglePlayerWorldNamesFromFilesystem();
        worldNames.forEach(saveName -> quickPlaySinglePlayerWorld.addItem(new ComboItem<>(saveName, saveName)));

        if (!worldNames.isEmpty()) {
            final int selectedWorldFolderNameIndex = worldNames.indexOf(quickPlay.worldName);
            quickPlaySinglePlayerWorld.setSelectedIndex(
                    selectedWorldFolderNameIndex != -1 ? selectedWorldFolderNameIndex : 0);
        }
        quickPlaySinglePlayerWorldRow = SettingRows.row(GetText.tr("World"),
                GetText.tr("The single player world to load once the game has started"),
                SettingRows.dropdown(quickPlaySinglePlayerWorld));

        quickPlayRealmId = new JTextField(13);
        quickPlayRealmId.putClientProperty("JTextField.showClearButton", true);
        quickPlayRealmId.putClientProperty("JTextField.clearCallback", (Runnable) () -> quickPlayRealmId.setText(""));
        quickPlayRealmId.setText(quickPlay.realmId);
        ModernDialogs.preferWidth(quickPlayRealmId, SettingRows.CONTROL_WIDTH);
        quickPlayRealmIdRow = SettingRows.row(GetText.tr("Realm ID"),
                GetText.tr("The ID of the realm to join once the game has started"), quickPlayRealmId);

        SettingRows.addSection(this, SettingRows.section(GetText.tr("Quick play"),
                SettingRows.row(GetText.tr("Start in"),
                        GetText.tr("Skip the title screen and go straight into a world, server or realm"),
                        SettingRows.dropdown(quickPlayType)),
                quickPlayServerAddressRow, quickPlaySinglePlayerWorldRow, quickPlayRealmIdRow));

        // Show only the input for the selected quick play type
        showInputForTheSelectedQuickPlayOption();
    }

    /**
     * A helper method to set the correct visibility for all the quick play inputs
     */
    public void showInputForTheSelectedQuickPlayOption() {
        QuickPlayOption quickPlayOption = ((ComboItem<QuickPlayOption>) quickPlayType.getSelectedItem()).getValue();
        switch (quickPlayOption) {
            case disabled:
                // Hide all
                setQuickPlayInputsVisibility(false, false, false);
                break;
            case multiPlayer:
                // Only show the server address input field
                setQuickPlayInputsVisibility(true, false, false);
                break;
            case singlePlayer:
                // Only show the select world dropdown
                setQuickPlayInputsVisibility(false, true, false);
                break;
            case realm:
                setQuickPlayInputsVisibility(false, false, true);
                break;
        }
    }

    /**
     * A helper method to set the visibility for different quick play inputs
     * (multiplayer, single player, realms)
     */
    public void setQuickPlayInputsVisibility(
            boolean serverAddress,
            boolean singlePlayerWorld,
            boolean realmId) {
        quickPlayServerAddressRow.setVisible(serverAddress);
        quickPlaySinglePlayerWorldRow.setVisible(singlePlayerWorld);
        quickPlayRealmIdRow.setVisible(realmId);
        revalidate();
        repaint();
    }

    public boolean isValidQuickPlayOptionValue() {
        QuickPlayOption quickPlayOption = ((ComboItem<QuickPlayOption>) quickPlayType.getSelectedItem()).getValue();
        switch (quickPlayOption) {
            case disabled:
                return true;
            case singlePlayer:
                if (quickPlaySinglePlayerWorld.getSelectedItem() == null) {
                    DialogManager.okDialog().setTitle(GetText.tr("Invalid Input"))
                            .setContent(new HTMLBuilder().center()
                                    .text(GetText.tr("You don't have any single player worlds yet on this instance."))
                                    .build())
                            .setType(DialogManager.ERROR).show();
                    return false;
                }
                return true;
            case multiPlayer:
                if (quickPlayServerAddress.getText().isEmpty()) {
                    DialogManager.okDialog().setTitle(GetText.tr("Invalid Input"))
                            .setContent(new HTMLBuilder().center()
                                    .text(GetText.tr("The server address is empty."))
                                    .build())
                            .setType(DialogManager.ERROR).show();
                    return false;
                }
                if (!ValidationUtils.isValidMinecraftServerAddress(quickPlayServerAddress.getText())) {
                    DialogManager.okDialog().setTitle(GetText.tr("Invalid Input"))
                            .setContent(new HTMLBuilder().center()
                                    .text(GetText.tr("The entered server address is invalid."))
                                    .build())
                            .setType(DialogManager.ERROR).show();
                    return false;
                }
                return true;
            case realm:
                if (quickPlayRealmId.getText().isEmpty()) {
                    DialogManager.okDialog().setTitle(GetText.tr("Invalid Input"))
                            .setContent(new HTMLBuilder().center()
                                    .text(GetText.tr("The realm id is empty."))
                                    .build())
                            .setType(DialogManager.ERROR).show();
                    return false;
                }
                return true;
        }
        return true;
    }

    public void saveSettings() {
        this.instance.launcher.account = ((ComboItem<String>) account.getSelectedItem()).getValue();
        QuickPlayOption quickPlayOption = ((ComboItem<QuickPlayOption>) quickPlayType.getSelectedItem()).getValue();
        this.instance.launcher.quickPlay = new QuickPlay(
                quickPlayOption == QuickPlayOption.multiPlayer ? quickPlayServerAddress.getText() : null,
                quickPlayOption == QuickPlayOption.singlePlayer
                        ? ((ComboItem<String>) quickPlaySinglePlayerWorld.getSelectedItem()).getValue()
                        : null,
                quickPlayOption == QuickPlayOption.realm ? quickPlayRealmId.getText() : null);
    }

}
