import { useState, useEffect } from "react";
import { invoke } from "@tauri-apps/api/core";
import { Button } from "../ui/Button";
import { Card } from "../ui/Card";
import { LoadingSpinner } from "../ui/LoadingSpinner";
import { useNotification } from "../../hooks/useNotification";
import { Skill, SkillsMarketplace, InstalledSkill } from "../../types/skills";
import { SkillDetailsDialog } from "./SkillDetailsDialog";

export const SkillsPage = () => {
  const [marketplace, setMarketplace] = useState<SkillsMarketplace | null>(
    null
  );
  const [installedSkills, setInstalledSkills] = useState<InstalledSkill[]>([]);
  const [loading, setLoading] = useState(true);
  const [refreshing, setRefreshing] = useState(false);
  const [downloading, setDownloading] = useState<Set<string>>(new Set());
  const [selectedSkill, setSelectedSkill] = useState<Skill | null>(null);
  const [searchQuery, setSearchQuery] = useState("");
  const [selectedTag, setSelectedTag] = useState<string>("");
  const [showInstalledOnly, setShowInstalledOnly] = useState(false);
  const [currentPage, setCurrentPage] = useState(1);
  const itemsPerPage = 24;
  const { showError, showSuccess } = useNotification();

  useEffect(() => {
    loadData();
  }, []);

  const loadData = async () => {
    setLoading(true);
    try {
      const [marketplaceData, installed] = await Promise.all([
        invoke<SkillsMarketplace>("fetch_skills_marketplace"),
        invoke<InstalledSkill[]>("get_installed_skills"),
      ]);
      setMarketplace(marketplaceData);
      setInstalledSkills(installed);
    } catch (error) {
      console.error("Failed to load skills:", error);
      showError("Failed to load skills marketplace");
    } finally {
      setLoading(false);
    }
  };

  const handleRefresh = async () => {
    setRefreshing(true);
    try {
      const [marketplaceData, installed] = await Promise.all([
        invoke<SkillsMarketplace>("refresh_skills_marketplace"),
        invoke<InstalledSkill[]>("get_installed_skills"),
      ]);
      setMarketplace(marketplaceData);
      setInstalledSkills(installed);
      showSuccess("Marketplace refreshed successfully");
    } catch (error) {
      console.error("Failed to refresh skills:", error);
      showError("Failed to refresh skills marketplace");
    } finally {
      setRefreshing(false);
    }
  };

  const handleDownload = async (skill: Skill) => {
    setDownloading((prev) => new Set(prev).add(skill.id));
    try {
      await invoke("download_skill", { skill });
      await loadData();
      showSuccess(`Successfully downloaded ${skill.title}`);
    } catch (error) {
      console.error("Failed to download skill:", error);
      showError(`Failed to download skill: ${error}`);
    } finally {
      setDownloading((prev) => {
        const next = new Set(prev);
        next.delete(skill.id);
        return next;
      });
    }
  };

  const handleDelete = async (skillId: string) => {
    try {
      await invoke("delete_skill", { skillId });
      await loadData();
      showSuccess("Skill deleted successfully");
    } catch (error) {
      console.error("Failed to delete skill:", error);
      showError(`Failed to delete skill: ${error}`);
    }
  };

  const isInstalled = (skillId: string) => {
    return installedSkills.some((s) => s.skill.id === skillId);
  };

  const getAllTags = () => {
    if (!marketplace) return [];
    const tags = new Set<string>();
    marketplace.skills.forEach((skill) => {
      skill.tags.forEach((tag) => tags.add(tag));
    });
    return Array.from(tags).sort();
  };

  const filteredSkills =
    marketplace?.skills.filter((skill) => {
      const matchesSearch =
        !searchQuery ||
        skill.title.toLowerCase().includes(searchQuery.toLowerCase()) ||
        skill.description.toLowerCase().includes(searchQuery.toLowerCase()) ||
        skill.author.toLowerCase().includes(searchQuery.toLowerCase());

      const matchesTag = !selectedTag || skill.tags.includes(selectedTag);

      const matchesInstalled = !showInstalledOnly || isInstalled(skill.id);

      return matchesSearch && matchesTag && matchesInstalled;
    }) || [];

  // Pagination
  const totalPages = Math.ceil(filteredSkills.length / itemsPerPage);
  const startIndex = (currentPage - 1) * itemsPerPage;
  const paginatedSkills = filteredSkills.slice(
    startIndex,
    startIndex + itemsPerPage
  );

  // Reset to page 1 when search/filter changes
  useEffect(() => {
    setCurrentPage(1);
  }, [searchQuery, selectedTag, showInstalledOnly]);

  if (loading) {
    return (
      <div className="flex items-center justify-center h-full">
        <LoadingSpinner />
      </div>
    );
  }

  return (
    <div className="h-full flex flex-col bg-gray-50 dark:bg-gray-900">
      {/* Header */}
      <div className="flex-shrink-0 bg-white dark:bg-gray-800 border-b border-gray-200 dark:border-gray-700 p-4">
        <div className="flex items-center justify-between mb-4">
          <h1 className="text-2xl font-bold text-gray-900 dark:text-white">
            Skills Marketplace
          </h1>
          <div className="flex items-center gap-2">
            <span className="text-sm text-gray-500 dark:text-gray-400">
              {installedSkills.length} installed · {filteredSkills.length}{" "}
              available
            </span>
            <Button
              onClick={() => setShowInstalledOnly(!showInstalledOnly)}
              variant={showInstalledOnly ? "default" : "outline"}
              size="sm"
            >
              {showInstalledOnly ? "Show All" : "Installed Only"}
            </Button>
            <Button
              onClick={handleRefresh}
              variant="outline"
              size="sm"
              disabled={refreshing}
            >
              {refreshing ? "Refreshing..." : "Refresh"}
            </Button>
          </div>
        </div>

        {/* Search and Filter */}
        <div className="flex gap-4">
          <input
            type="text"
            placeholder="Search skills..."
            value={searchQuery}
            onChange={(e) => setSearchQuery(e.target.value)}
            className="flex-1 px-4 py-2 border border-gray-300 dark:border-gray-600 rounded-lg bg-white dark:bg-gray-700 text-gray-900 dark:text-white focus:outline-none focus:ring-2 focus:ring-blue-500"
          />
          <select
            value={selectedTag}
            onChange={(e) => setSelectedTag(e.target.value)}
            className="px-4 py-2 border border-gray-300 dark:border-gray-600 rounded-lg bg-white dark:bg-gray-700 text-gray-900 dark:text-white focus:outline-none focus:ring-2 focus:ring-blue-500"
          >
            <option value="">All Tags</option>
            {getAllTags().map((tag) => (
              <option key={tag} value={tag}>
                {tag}
              </option>
            ))}
          </select>
        </div>
      </div>

      {/* Skills Grid */}
      <div className="flex-1 overflow-y-auto p-6">
        <div className="grid grid-cols-1 md:grid-cols-2 lg:grid-cols-3 gap-4">
          {paginatedSkills.map((skill) => {
            const installed = isInstalled(skill.id);
            const isDownloading = downloading.has(skill.id);

            return (
              <Card key={skill.id} className="flex flex-col h-full">
                <div className="flex-1">
                  <div className="flex items-start justify-between mb-2">
                    <h3 className="text-lg font-semibold text-gray-900 dark:text-white line-clamp-2">
                      {skill.title}
                    </h3>
                    {installed && (
                      <span className="ml-2 px-2 py-1 text-xs bg-green-100 dark:bg-green-900 text-green-800 dark:text-green-200 rounded">
                        Installed
                      </span>
                    )}
                  </div>

                  <p className="text-sm text-gray-600 dark:text-gray-400 mb-3 line-clamp-3">
                    {skill.description}
                  </p>

                  <div className="flex items-center gap-2 text-xs text-gray-500 dark:text-gray-400 mb-3">
                    <span>by {skill.author || "Unknown"}</span>
                    {skill.license && (
                      <>
                        <span>•</span>
                        <span>{skill.license}</span>
                      </>
                    )}
                  </div>

                  {skill.tags.length > 0 && (
                    <div className="flex flex-wrap gap-1 mb-3">
                      {skill.tags.slice(0, 3).map((tag) => (
                        <span
                          key={tag}
                          className="px-2 py-0.5 text-xs bg-gray-100 dark:bg-gray-700 text-gray-700 dark:text-gray-300 rounded"
                        >
                          {tag}
                        </span>
                      ))}
                      {skill.tags.length > 3 && (
                        <span className="px-2 py-0.5 text-xs text-gray-500 dark:text-gray-400">
                          +{skill.tags.length - 3}
                        </span>
                      )}
                    </div>
                  )}
                </div>

                <div className="flex gap-2 mt-4">
                  <Button
                    onClick={() => setSelectedSkill(skill)}
                    variant="outline"
                    className="flex-1"
                    size="sm"
                  >
                    Details
                  </Button>
                  {installed ? (
                    <Button
                      onClick={() => handleDelete(skill.id)}
                      variant="outline"
                      className="flex-1 text-red-600 hover:text-red-700 hover:bg-red-50 dark:hover:bg-red-900/20"
                      size="sm"
                    >
                      Delete
                    </Button>
                  ) : (
                    <Button
                      onClick={() => handleDownload(skill)}
                      disabled={isDownloading}
                      className="flex-1"
                      size="sm"
                    >
                      {isDownloading ? (
                        <div className="flex items-center justify-center gap-2">
                          <LoadingSpinner size="sm" />
                          <span>Downloading...</span>
                        </div>
                      ) : (
                        "Download"
                      )}
                    </Button>
                  )}
                </div>
              </Card>
            );
          })}
        </div>

        {filteredSkills.length === 0 && (
          <div className="text-center py-12">
            <p className="text-gray-500 dark:text-gray-400">
              No skills found matching your search criteria.
            </p>
          </div>
        )}

        {/* Pagination */}
        {totalPages > 1 && (
          <div className="flex items-center justify-center gap-2 mt-8 pb-4">
            <Button
              onClick={() => setCurrentPage((p) => Math.max(1, p - 1))}
              disabled={currentPage === 1}
              variant="outline"
              size="sm"
            >
              Previous
            </Button>
            <span className="text-sm text-gray-600 dark:text-gray-400 px-4">
              Page {currentPage} of {totalPages}
            </span>
            <Button
              onClick={() => setCurrentPage((p) => Math.min(totalPages, p + 1))}
              disabled={currentPage === totalPages}
              variant="outline"
              size="sm"
            >
              Next
            </Button>
          </div>
        )}
      </div>

      {/* Details Dialog */}
      {selectedSkill && (
        <SkillDetailsDialog
          skill={selectedSkill}
          isInstalled={isInstalled(selectedSkill.id)}
          onClose={() => setSelectedSkill(null)}
          onDownload={() => {
            handleDownload(selectedSkill);
            setSelectedSkill(null);
          }}
          onDelete={() => {
            handleDelete(selectedSkill.id);
            setSelectedSkill(null);
          }}
        />
      )}
    </div>
  );
};
