// The app puts why Meka Studio could not start in the address, as `#error=<message>`.
const status = document.getElementById('status');
function showError() {
  const error = /^#error=(.*)$/.exec(location.hash);
  if (!error) return;
  status.textContent = decodeURIComponent(error[1].replace(/\+/g, ' '));
  status.className = 'error';
}
window.addEventListener('hashchange', showError);
showError();
