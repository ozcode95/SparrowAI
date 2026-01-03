export interface Skill {
  id: string;
  title: string;
  description: string;
  author: string;
  author_url: string;
  author_github: string;
  license: string;
  license_url: string;
  skill_url: string;
  skill_download_url: string;
  tags: string[];
}

export interface SkillsMarketplace {
  skills: Skill[];
}

export interface InstalledSkill {
  skill: Skill;
  installed_at: string;
  local_path: string;
}
