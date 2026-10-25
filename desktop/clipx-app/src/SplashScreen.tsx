export function SplashScreen() {
  return (
    <div className="startup-splash" role="status" aria-label="ClipX is starting">
      <div className="startup-splash-card">
        <img className="startup-splash-logo" src="/clipx-icon.png" alt="ClipX" />
        <div className="startup-splash-name">ClipX</div>
        <div className="startup-splash-caption">Getting things ready</div>
        <div className="startup-splash-progress" aria-hidden="true">
          <span />
        </div>
      </div>
    </div>
  );
}
