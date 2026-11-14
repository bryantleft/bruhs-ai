const { IOSConfig, withXcodeProject } = require('expo/config-plugins');

/** @type {import('expo/config-plugins').ConfigPlugin} */
module.exports = function withLocalIosSigning(config) {
  return withXcodeProject(config, (config) => {
    const team = config.ios?.appleTeamId;
    if (!team || !/^[A-Z0-9]{10}$/.test(team)) {
      throw new Error('Set ios.appleTeamId before configuring local iOS signing.');
    }
    const project = config.modResults;
    for (const [targetId, target] of IOSConfig.Target.findSignableTargets(project)) {
      for (const [, build] of IOSConfig.XcodeUtils.getBuildConfigurationsForListId(
        project,
        target.buildConfigurationList,
      )) {
        build.buildSettings.DEVELOPMENT_TEAM = team;
        build.buildSettings.CODE_SIGN_STYLE = 'Automatic';
        // Simulator builds use local ad-hoc signing without contacting an Apple
        // account. Only device SDK builds select an iOS development identity.
        build.buildSettings.CODE_SIGN_IDENTITY = '"-"';
        build.buildSettings['"CODE_SIGN_IDENTITY[sdk=iphoneos*]"'] = '"Apple Development"';
        delete build.buildSettings.PROVISIONING_PROFILE;
        delete build.buildSettings.PROVISIONING_PROFILE_SPECIFIER;
      }
      for (const [, section] of Object.entries(
        IOSConfig.XcodeUtils.getProjectSection(project),
      ).filter(IOSConfig.XcodeUtils.isNotComment)) {
        section.attributes ??= {};
        section.attributes.TargetAttributes ??= {};
        section.attributes.TargetAttributes[targetId] ??= {};
        section.attributes.TargetAttributes[targetId].DevelopmentTeam = team;
        section.attributes.TargetAttributes[targetId].ProvisioningStyle = 'Automatic';
      }
    }
    return config;
  });
};
