import { ComputerUseControl } from "./ComputerUseControl";

export function PluginsView({
  onOpenModels,
  onOpenBilling,
}: {
  onOpenModels: () => void;
  onOpenBilling: () => void;
}) {
  return (
    <section className="plugins-view" aria-labelledby="plugins-view-heading">
      <header className="plugins-view-header">
        <h1 id="plugins-view-heading">Plugins</h1>
        <p>Give June carefully scoped ways to work in other apps and services.</p>
      </header>
      <div className="plugins-grid">
        <article className="plugin-tile">
          <ComputerUseControl onOpenModels={onOpenModels} onOpenBilling={onOpenBilling} />
        </article>
      </div>
    </section>
  );
}
